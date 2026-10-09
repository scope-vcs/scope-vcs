import io
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock, patch
from urllib.parse import parse_qs, urlsplit

from triage.policy import timestamp
from triage.sources import Budget, SourceError, browser, github_issues, github_runs, railway


START = timestamp('2026-10-08T12:00:00Z')
END = timestamp('2026-10-08T12:00:10Z')
PRIVATE = 'private-token-and-repository-path'
TRACE = 'a' * 32
RELEASE = 'b' * 40
MANIFEST = json.loads((Path(__file__).resolve().parents[2] / '.github/deployment-services.json').read_text())
API_ID = MANIFEST['services']['api']['id']


def result(data, ndjson=False):
    output = '\n'.join(json.dumps(row) for row in data) if ndjson else json.dumps(data)
    return subprocess.CompletedProcess([], 0, output, '')


def tracing_status():
    return {'services': [{'id': item['id'], 'tracingEnabled': True, 'lastServiceSpanAt': START}
                         for item in MANIFEST['services'].values()]}


def span(**changes):
    return {'traceId': TRACE, 'spanId': 'c' * 16, 'startedAt': START, 'statusCode': 'ERROR',
            'serviceId': API_ID, 'name': PRIVATE, 'deploymentId': 'deployment-1',
            'resourceAttributes': {'service.version': RELEASE, 'secret': PRIVATE},
            'spanAttributes': {'http.route': '/api/requests', 'secret': PRIVATE},
            'statusMessage': PRIVATE, 'events': [{'name': PRIVATE}], **changes}


def issue(number, **changes):
    return {'number': number, 'state': 'open', 'updated_at': START, 'labels': [{'name': 'bug'}],
            'title': PRIVATE, 'body': PRIVATE, 'user': {'login': PRIVATE}, **changes}


def run(**changes):
    return {'id': 11, 'workflow_id': 22, 'created_at': START, 'updated_at': START,
            'status': 'completed', 'conclusion': 'failure', 'run_attempt': 1,
            'path': '.github/workflows/ci.yml', 'head_sha': RELEASE,
            'display_title': PRIVATE, **changes}


class SourceTests(unittest.TestCase):
    def assert_private_absent(self, observations):
        self.assertNotIn(PRIVATE, json.dumps([item.record() for item in observations]))

    def test_railway_ndjson_trace_deduplication_filters_health_and_retains_safe_correlation(self):
        replies = [result(tracing_status()), result([{'traceId': TRACE}, {'traceId': TRACE}], True),
                   result([span(), span(spanId='d' * 16, statusCode='OK'),
                           span(spanId='e' * 16, spanAttributes={'http.route': '/healthz'})], True)]
        with patch('triage.sources.subprocess.run', side_effect=replies):
            observations = railway(START, END, MANIFEST['environments']['production']['environmentId'], Budget())
        self.assertEqual(len(observations), 1)
        observation = observations[0]
        self.assertEqual(observation.trace_id, TRACE)
        self.assertEqual(observation.release, RELEASE)
        self.assertEqual(observation.deployment, 'deployment-1')
        self.assertEqual(observation.service, 'api')
        self.assertEqual(observation.reference, 'railway:trace:' + TRACE)
        self.assert_private_absent(observations)
        with patch('triage.sources.subprocess.run', side_effect=[result(tracing_status()), result([{'traceId': TRACE}], True), result([span(spanId=PRIVATE)], True)]):
            with self.assertRaises(SourceError) as error:
                railway(START, END, MANIFEST['environments']['production']['environmentId'], Budget())
        self.assertEqual(error.exception.reason, 'invalid_response')

    def test_railway_full_trace_windows_split_and_deduplicate_boundary_trace(self):
        responses = [result(tracing_status()), result([{'traceId': TRACE}] * 500, True),
                     result([{'traceId': TRACE}], True), result([{'traceId': TRACE}], True),
                     result([span()], True)]
        with patch('triage.sources.subprocess.run', side_effect=responses) as transport:
            observations = railway(START, END, MANIFEST['environments']['production']['environmentId'], Budget())
        self.assertEqual(len(observations), 1)
        windows = [call.args[0] for call in transport.call_args_list if call.args[0][2] == 'list']
        intervals = [(args[args.index('--since') + 1], args[args.index('--until') + 1]) for args in windows]
        self.assertEqual(intervals[0], (START, END))
        self.assertEqual(intervals[1][0], START)
        self.assertEqual(intervals[1][1], intervals[2][0])
        self.assertEqual(intervals[2][1], END)

    def test_railway_empty_ndjson_is_healthy_only_with_received_service_coverage(self):
        with patch('triage.sources.subprocess.run', side_effect=[result(tracing_status()), result([], True)]):
            self.assertEqual(railway(START, END, MANIFEST['environments']['production']['environmentId'], Budget()), [])
        status = tracing_status()
        status['services'][0]['lastServiceSpanAt'] = None
        with patch('triage.sources.subprocess.run', return_value=result(status)) as transport:
            with self.assertRaises(SourceError) as error:
                railway(START, END, MANIFEST['environments']['production']['environmentId'], Budget())
        self.assertEqual(error.exception.reason, 'configuration')
        self.assertEqual(transport.call_count, 1)

    def test_railway_caps_cannot_return_a_partial_success(self):
        cases = [
            (START, timestamp('2026-10-08T12:00:01Z'), [result(tracing_status()), result([{'traceId': TRACE}] * 500, True)]),
            (START, END, [result(tracing_status()), result([{'traceId': TRACE}], True), result([span()] * 2000, True)]),
        ]
        for start, end, responses in cases:
            with self.subTest(end=end), patch('triage.sources.subprocess.run', side_effect=responses):
                with self.assertRaises(SourceError) as error:
                    railway(start, end, MANIFEST['environments']['production']['environmentId'], Budget())
            self.assertEqual(error.exception.reason, 'incomplete')

    def test_railway_window_comparison_accepts_equivalent_iso_offsets(self):
        responses = [result(tracing_status()), result([{'traceId': TRACE}], True),
                     result([span(startedAt='2026-10-08T07:00:10-05:00')], True)]
        with patch('triage.sources.subprocess.run', side_effect=responses):
            observations = railway('2026-10-08T12:00:00Z', '2026-10-08T12:00:10Z', MANIFEST['environments']['production']['environmentId'], Budget())
        self.assertEqual(len(observations), 1)
        self.assertEqual(observations[0].occurred_at, END)

    def test_github_issue_pages_filter_prs_labels_and_window_without_raw_issue_content(self):
        first = [issue(1), issue(2, pull_request={}), issue(3, labels=[{'name': 'enhancement'}]), issue(4, state='closed')]
        first.extend(issue(number, labels=[]) for number in range(5, 101))
        second = [issue(101, labels=[{'name': 'release-flake'}]),
                  issue(102, updated_at=timestamp('2026-10-08T12:00:11Z'))]
        with patch('triage.sources.subprocess.run', side_effect=[result(first), result(second)]) as transport:
            observations = github_issues(START, END, Budget())
        self.assertEqual([item.operation for item in observations], ['issue:1', 'issue:101'])
        self.assertFalse(observations[0].release_owned)
        self.assertTrue(observations[1].release_owned)
        self.assert_private_absent(observations)
        pages = [parse_qs(urlsplit(call.args[0][-1]).query)['page'][0] for call in transport.call_args_list]
        self.assertEqual(pages, ['1', '2'])

    def test_github_failed_attempts_preserve_attempt_identity_and_release_ownership(self):
        responses = [result({'total_count': 1, 'workflow_runs': [run(run_attempt=3)]}),
                     result(run(conclusion='success')), result(run(conclusion='timed_out', updated_at=END)),
                     result(run(path='.github/workflows/release.yml@refs/heads/main'))]
        with patch('triage.sources.subprocess.run', side_effect=responses):
            observations = github_runs(START, END, Budget())
        self.assertEqual([item.event_id for item in observations], ['run:11:2', 'run:11:3'])
        self.assertEqual([item.signature for item in observations], ['timed_out', 'failure'])
        self.assertEqual([item.release_owned for item in observations], [False, True])
        self.assertEqual(observations[0].occurred_at, END)
        self.assertEqual(observations[0].indexed_at, START)
        self.assertTrue(observations[1].reference.endswith('/attempts/3'))
        self.assert_private_absent(observations)

    def test_github_pending_runs_and_search_cap_abort_without_completed_coverage(self):
        responses = [
            {'total_count': 1000, 'workflow_runs': []},
            {'total_count': 1, 'workflow_runs': [run(status='in_progress')]},
        ]
        for response in responses:
            with self.subTest(response=response), patch('triage.sources.subprocess.run', return_value=result(response)):
                with self.assertRaises(SourceError) as error:
                    github_runs(START, END, Budget())
            self.assertEqual(error.exception.reason, 'incomplete')

    def test_browser_hogql_rows_hash_routes_and_drop_raw_payload_and_credentials(self):
        payload = {'results': [['event-1', START, 'type_error', 'promise', PRIVATE, RELEASE]],
                   'raw_payload': PRIVATE}
        opener = Mock()
        opener.open.return_value = io.BytesIO(json.dumps(payload).encode())
        with patch.dict('os.environ', {'POSTHOG_PROJECT_ID': '123', 'POSTHOG_PERSONAL_API_KEY': PRIVATE,
                                      'POSTHOG_APP_HOST': 'https://us.posthog.com'}), \
                patch('triage.sources.urllib.request.build_opener', return_value=opener):
            observations = browser(START, END, Budget())
        self.assertEqual(len(observations), 1)
        self.assertEqual(observations[0].signature, 'type_error:promise')
        self.assertEqual(observations[0].release, RELEASE)
        self.assertEqual(observations[0].reference, 'posthog:event:event-1')
        self.assert_private_absent(observations)

    def test_browser_incomplete_query_and_invalid_error_taxonomy_fail_closed(self):
        for payload, reason in [({'error': PRIVATE, 'results': []}, 'incomplete'),
                                ({'results': [['event-1', START, PRIVATE, 'promise', 'request', RELEASE]]}, 'invalid_response')]:
            opener = Mock()
            opener.open.return_value = io.BytesIO(json.dumps(payload).encode())
            with self.subTest(reason=reason), \
                    patch.dict('os.environ', {'POSTHOG_PROJECT_ID': '123', 'POSTHOG_PERSONAL_API_KEY': PRIVATE}), \
                    patch('triage.sources.urllib.request.build_opener', return_value=opener):
                with self.assertRaises(SourceError) as error:
                    browser(START, END, Budget())
            self.assertEqual(error.exception.reason, reason)
            self.assertNotIn(PRIVATE, str(error.exception))

    def test_browser_capped_query_splits_time_ranges_before_returning_observations(self):
        first = ['event-1', START, 'type_error', 'promise', PRIVATE, RELEASE]
        last = ['event-2', END, 'range_error', 'window', PRIVATE, RELEASE]
        payloads = [{'results': [first] * 500}, {'results': [first]}, {'results': [last]}]
        opener = Mock()
        opener.open.side_effect = [io.BytesIO(json.dumps(payload).encode()) for payload in payloads]
        with patch.dict('os.environ', {'POSTHOG_PROJECT_ID': '123', 'POSTHOG_PERSONAL_API_KEY': PRIVATE}), \
                patch('triage.sources.urllib.request.build_opener', return_value=opener):
            observations = browser(START, END, Budget())
        self.assertEqual([item.event_id for item in observations], ['event-1', 'event-2'])
        self.assertEqual(opener.open.call_count, 3)
        self.assert_private_absent(observations)

    def test_malformed_ndjson_cannot_leak_transport_body(self):
        response = subprocess.CompletedProcess([], 0, '{"message":"' + PRIVATE, '')
        with patch('triage.sources.subprocess.run', return_value=response):
            with self.assertRaises(SourceError) as error:
                Budget().command(['railway', 'trace', 'list', '--json'], ndjson=True)
        self.assertEqual(error.exception.reason, 'invalid_response')
        self.assertNotIn(PRIVATE, str(error.exception))

    def test_transport_failures_and_budget_exhaustion_never_expose_provider_output(self):
        outcomes = [subprocess.CompletedProcess([], 1, PRIVATE, PRIVATE),
                    subprocess.TimeoutExpired(['railway'], 1, output=PRIVATE)]
        for outcome in outcomes:
            transport = Mock(side_effect=outcome) if isinstance(outcome, Exception) else Mock(return_value=outcome)
            with self.subTest(outcome=type(outcome).__name__), patch('triage.sources.subprocess.run', transport):
                with self.assertRaises(SourceError) as error:
                    Budget().command(['railway', 'trace', 'status'])
            self.assertEqual(error.exception.reason, 'unavailable')
            self.assertNotIn(PRIVATE, str(error.exception))
        budget = Budget()
        budget.remaining = 0
        with patch('triage.sources.subprocess.run') as transport:
            with self.assertRaises(SourceError) as error:
                budget.command(['railway', 'trace', 'status'])
        self.assertEqual(error.exception.reason, 'incomplete')
        self.assertEqual(transport.call_count, 0)


if __name__ == '__main__':
    unittest.main()
