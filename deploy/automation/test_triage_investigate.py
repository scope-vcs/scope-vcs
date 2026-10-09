import copy
import json
import unittest

from triage.investigate import delegation_request, render_draft, select_target, status_request, validate_result
from triage.policy import Observation, evidence_packet


def packet():
    observation = Observation(source='railway', event_id='event-1', occurred_at='2026-10-08T12:00:00Z', environment='production', service='backend', operation='request', signature='server-error', reference='railway:trace:' + 'a' * 32, release='b' * 40)
    return evidence_packet({'fingerprint': observation.fingerprint}, [observation.record()])


def diagnosis():
    return {'facts': [{'observation': 'event-1', 'finding': 'observed_failure'}], 'hypothesis': 'application_defect', 'confidence': 'low', 'reproduction': 'unknown', 'impact': 'availability', 'severity': 'medium', 'owner': 'event-1', 'next_action': 'attempt_reproduction', 'duplicate_candidates': []}


class InvestigationTests(unittest.TestCase):
    def test_closed_output_contract_rejects_untrusted_prose_and_unsupported_claims(self):
        evidence = packet()
        self.assertEqual(validate_result(json.dumps(diagnosis()), evidence), diagnosis())
        mutations = [
            {'hypothesis': 'secret-token-value'},
            {'facts': [{'observation': 'event-1', 'finding': 'secret-token-value'}]},
            {'facts': [{'observation': 'missing', 'finding': 'observed_failure'}]},
            {'facts': [{'observation': 'event-1', 'finding': 'observed_failure', 'log': 'secret-token-value'}]},
            {'facts': []},
            {'owner': 'https://private.example/path'},
            {'duplicate_candidates': ['https://github.com/scope-vcs/scope-vcs/issues/123']},
            {'reproduction': 'reproduced'},
            {'reproduction': 'not_reproduced'},
            {'hypothesis': 'unknown', 'confidence': 'high'},
            {'confidence': 'certain'},
            {'private_payload': 'secret-token-value'},
        ]
        for changes in mutations:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                validate_result({**diagnosis(), **changes}, evidence)
        with self.assertRaises(ValueError):
            validate_result(json.dumps(diagnosis())[:-1] + ',"confidence":"high"}', evidence)

    def test_issue_candidates_must_resolve_to_packet_references(self):
        evidence = packet()
        evidence['observations'][0]['reference'] = 'https://github.com/scope-vcs/scope-vcs/issues/37'
        result = {**diagnosis(), 'duplicate_candidates': ['https://github.com/scope-vcs/scope-vcs/issues/37']}
        self.assertIn('issues/37', render_draft(evidence, result))
        result['duplicate_candidates'].append(result['duplicate_candidates'][0])
        with self.assertRaises(ValueError):
            validate_result(result, evidence)

    def test_draft_separates_observations_from_inference_and_omits_raw_selectors(self):
        evidence = packet()
        evidence['observations'][0].update(event_id='credential-in-selector', operation='private-file-path', signature='secret-in-signature')
        result = diagnosis()
        result['facts'][0]['observation'] = 'credential-in-selector'
        result['owner'] = 'credential-in-selector'
        draft = render_draft(evidence, result)
        self.assertIn('scope-incident:' + evidence['incident'], draft)
        self.assertIn('recorded failure; source railway; environment production', draft)
        self.assertIn('Hypothesis: application_defect', draft)
        self.assertIn('Suggested severity: medium', draft)
        self.assertIn('Reproduction: unknown', draft)
        self.assertIn('affected-user count, and duration remain unverified', draft)
        self.assertIn('service associated with evidence 1', draft)
        self.assertIn('attempt reproduction', draft)
        for private in ('credential-in-selector', 'private-file-path', 'secret-in-signature'):
            self.assertNotIn(private, draft)

    def test_request_identity_and_polling_survive_retries(self):
        claim = {'packet': packet(), 'request_id': 'incident-claim-1', 'task_id': 'persisted-task'}
        request = delegation_request(claim, 'provider', 'catalog-model', {'reasoning': 'medium'})
        self.assertEqual(request['clientRequestId'], 'incident-claim-1')
        self.assertEqual(request['mode'], 'async')
        self.assertEqual(request['runtimeMode'], 'approval-required')
        self.assertIn('Do not retrieve raw logs', request['task'])
        self.assertEqual(status_request(claim), {'taskId': 'persisted-task'})
        with self.assertRaises(ValueError):
            status_request({'task_id': None})
        claim['packet']['observations'] *= 21
        with self.assertRaises(ValueError):
            delegation_request(claim, 'provider', 'catalog-model')

    def test_live_catalog_limits_models_options_and_cross_provider_execution(self):
        catalog = {'inheritedProviderInstanceId': 'provider', 'inheritedModel': 'new-model', 'providers': [{'providerInstanceId': 'provider', 'canRunChildTask': True, 'canRunCrossProviderChildTask': False, 'models': [{'id': 'new-model', 'options': [{'id': 'reasoning', 'type': 'select', 'options': [{'id': 'medium'}]}, {'id': 'fast', 'type': 'boolean'}]}]}]}
        target = select_target(catalog, options={'reasoning': 'medium', 'fast': False})
        self.assertEqual(target['model'], 'new-model')
        for choices in ({'model': 'obsolete-model'}, {'options': {'reasoning': 'unlisted'}}, {'options': {'fast': 'false'}}, {'options': {'unlisted': True}}):
            with self.subTest(choices=choices), self.assertRaises(ValueError):
                select_target(catalog, **choices)
        other = copy.deepcopy(catalog['providers'][0])
        other['providerInstanceId'] = 'other'
        catalog['providers'].append(other)
        with self.assertRaises(ValueError):
            select_target(catalog, provider='other')
        other['canRunCrossProviderChildTask'] = True
        self.assertEqual(select_target(catalog, provider='other')['providerInstanceId'], 'other')


if __name__ == '__main__':
    unittest.main()
