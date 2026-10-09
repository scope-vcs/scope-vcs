import hashlib
import hmac
import json
import unittest
from unittest.mock import MagicMock, patch

import deployment_webhook as webhook


RUN = {"id": 123, "run_attempt": 1, "status": "completed", "head_branch": "main", "event": "workflow_dispatch",
       "repository": {"full_name": "scope-vcs/scope-vcs"}, "path": ".github/workflows/release.yml"}
EVENT = {"action": "completed", "repository": {"full_name": "scope-vcs/scope-vcs"}, "workflow_run": RUN}
URL = "https://relay.t3.codes/v1/hooks/test/route"


class WebhookTests(unittest.TestCase):
    def test_manual_probe_reads_an_existing_release_without_dispatching_one(self):
        with patch.object(webhook, "github", return_value=RUN) as request:
            event = webhook.source_event({"inputs": {"run_id": "123"}}, "workflow_dispatch")
        self.assertEqual(event, EVENT)
        request.assert_called_once_with("actions/runs/123")

    def test_sender_signs_exact_bytes_and_preserves_delivery_identity(self):
        opener = MagicMock()
        opener.open.return_value.__enter__.return_value.status = 202
        with patch.object(webhook, "github", return_value=RUN), \
                patch.object(webhook.urllib.request, "build_opener", return_value=opener):
            first = webhook.forward(EVENT, URL, "fixture-secret")
            second = webhook.forward(EVENT, URL, "fixture-secret")
        request = opener.open.call_args.args[0]
        signature = hmac.new(b"fixture-secret", request.data, hashlib.sha256).hexdigest()
        self.assertEqual(request.get_header("X-hub-signature-256"), "sha256=" + signature)
        self.assertEqual(first, second)
        self.assertEqual(json.loads(request.data)["run_id"], 123)
        self.assertNotIn("fixture-secret", request.data.decode())

    def test_untrusted_or_unrelated_workflows_never_contact_the_webhook(self):
        for change in ({"head_branch": "feature"}, {"path": ".github/workflows/validate.yml"},
                       {"repository": {"full_name": "fork/repo"}}):
            with self.subTest(change=change), patch.object(webhook.urllib.request, "build_opener") as connect:
                self.assertEqual(webhook.forward(EVENT | {"workflow_run": RUN | change}, URL, "fixture-secret"), {"ignored": True})
            connect.assert_not_called()
        with patch.object(webhook, "github", return_value=RUN | {"head_branch": "feature"}), \
                patch.object(webhook.urllib.request, "build_opener") as connect:
            self.assertEqual(webhook.forward(EVENT, URL, "fixture-secret"), {"ignored": True})
        connect.assert_not_called()

    def test_credentials_cannot_be_sent_to_a_redirect_or_a_non_relay_destination(self):
        for url in ("http://relay.t3.codes/v1/hooks/test", "https://other.example/v1/hooks/test",
                    "https://relay.t3.codes/not-a-hook"):
            with self.subTest(url=url), patch.object(webhook, "github", return_value=RUN), \
                    patch.object(webhook.urllib.request, "build_opener") as connect:
                with self.assertRaises(ValueError):
                    webhook.forward(EVENT, url, "fixture-secret")
            connect.assert_not_called()


if __name__ == "__main__":
    unittest.main()
