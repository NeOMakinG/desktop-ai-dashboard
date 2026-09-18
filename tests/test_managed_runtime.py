import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import json
import unittest

from forma_runtime.contracts import Fault
from forma_runtime.managed import (
    PROTOCOL,
    Controller,
    model_configuration,
    model_scope,
    owned_path,
    response,
)

FRAME_ID = "11111111-1111-4111-8111-111111111111"
KEY_VALUE = "v" * 8


def base_model():
    return {
        "providerId": "builtin",
        "gatewayUrl": "http://127.0.0.1:8317/v1",
        "gatewayKey": KEY_VALUE,
        "models": [
            {"id": "model-a", "name": "Model A", "available": True},
            {"id": "model-b", "name": "Model B", "available": False, "reason": "offline"},
        ],
        "dailyLimit": 96,
        "configEpoch": "22222222-2222-4222-8222-222222222222",
    }


class TestProtocolConstant(unittest.TestCase):
    def test_protocol_marker_is_stable(self):
        self.assertEqual(PROTOCOL, "forma-managed-v1")


class TestModelConfiguration(unittest.TestCase):
    def test_valid_configuration_returns_independent_copy(self):
        original = base_model()
        model = model_configuration(original)
        self.assertEqual(model["gatewayKey"], KEY_VALUE)
        self.assertEqual(model["dailyLimit"], 96)
        self.assertIsNot(model, original)

    def test_missing_optional_fields_get_defaults(self):
        model = base_model()
        del model["dailyLimit"]
        del model["gatewayKey"]
        result = model_configuration(model)
        self.assertEqual(result["dailyLimit"], 192)
        self.assertEqual(result["gatewayKey"], "")

    def test_scheme_must_be_http_or_https(self):
        model = base_model()
        model["gatewayUrl"] = "ftp://127.0.0.1:8317/v1"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_userinfo_in_url_rejected(self):
        model = base_model()
        model["gatewayUrl"] = "https://user:pw@example.com/v1"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_query_or_fragment_in_url_rejected(self):
        model = base_model()
        model["gatewayUrl"] = "http://127.0.0.1:8317/v1?x=1"
        with self.assertRaises(Fault):
            model_configuration(model)
        model["gatewayUrl"] = "http://127.0.0.1:8317/v1#top"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_url_path_must_be_version_root(self):
        model = base_model()
        model["gatewayUrl"] = "http://127.0.0.1:8317/v1/extra"
        with self.assertRaises(Fault):
            model_configuration(model)
        model["gatewayUrl"] = "http://127.0.0.1:8317"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_empty_hostname_rejected(self):
        model = base_model()
        model["gatewayUrl"] = "http:///v1"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_out_of_range_port_rejected(self):
        model = base_model()
        model["gatewayUrl"] = "http://127.0.0.1:70000/v1"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_required_top_fields_must_exist(self):
        model = base_model()
        del model["providerId"]
        with self.assertRaises(Fault):
            model_configuration(model)
        model = base_model()
        del model["models"]
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_unknown_top_field_rejected(self):
        model = base_model()
        model["extraField"] = 1
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_daily_limit_bounds(self):
        model = base_model()
        model["dailyLimit"] = 0
        with self.assertRaises(Fault):
            model_configuration(model)
        model["dailyLimit"] = 193
        with self.assertRaises(Fault):
            model_configuration(model)
        model["dailyLimit"] = 192
        self.assertEqual(model_configuration(model)["dailyLimit"], 192)

    def test_model_catalog_entries_validated(self):
        model = base_model()
        model["models"][0]["available"] = "yes"
        with self.assertRaises(Fault):
            model_configuration(model)
        model = base_model()
        model["models"][0]["id"] = ""
        with self.assertRaises(Fault):
            model_configuration(model)
        model = base_model()
        model["models"][0]["junk"] = 1
        with self.assertRaises(Fault):
            model_configuration(model)
        model = base_model()
        model["models"][1]["id"] = "model-a"
        with self.assertRaises(Fault):
            model_configuration(model)

    def test_invalid_config_epoch_rejected(self):
        model = base_model()
        model["configEpoch"] = "not-a-uid"
        with self.assertRaises(Fault):
            model_configuration(model)


class TestModelScope(unittest.TestCase):
    def test_scope_carries_only_authority_fields(self):
        scope = model_scope(base_model())
        self.assertEqual(sorted(scope), ["configEpoch", "gatewayUrl", "providerId"])
        self.assertEqual(scope["providerId"], "builtin")

    def test_scope_of_partial_model_uses_none(self):
        self.assertEqual(
            model_scope({}),
            {"providerId": None, "gatewayUrl": None, "configEpoch": None},
        )


class TestOwnedPath(unittest.TestCase):
    def test_absolute_path_accepted(self):
        self.assertEqual(str(owned_path("/tmp/forma-profile")), "/tmp/forma-profile")

    def test_relative_path_rejected(self):
        with self.assertRaises(Fault):
            owned_path("relative/path")

    def test_control_characters_rejected(self):
        with self.assertRaises(Fault):
            owned_path("/tmp/bad\tname")

    def test_overlong_path_rejected(self):
        with self.assertRaises(Fault):
            owned_path("/" + "a" * 4096)


class TestControllerGate(unittest.TestCase):
    def setUp(self):
        self.controller = Controller()

    def test_frame_must_be_object(self):
        with self.assertRaises(Fault):
            self.controller.dispatch(["nope"])
        with self.assertRaises(Fault):
            self.controller.dispatch("text")

    def test_frame_id_must_be_uuid(self):
        with self.assertRaises(Fault):
            self.controller.dispatch({"id": "bad", "op": "lifecycle"})

    def test_operations_before_bootstrap_rejected(self):
        for op in ("lifecycle", "request", "shutdown", "mystery"):
            with self.assertRaises(Fault) as caught:
                self.controller.dispatch({"id": FRAME_ID, "op": op})
            self.assertEqual(caught.exception.code, "not_bootstrapped")

    def test_close_without_app_is_safe(self):
        self.controller.close()
        self.assertTrue(self.controller.shutdown)
        self.assertIsNone(self.controller.model)


class TestResponseEnvelope(unittest.TestCase):
    def test_fault_envelope_carries_frame_identity(self):
        raw = json.dumps({"id": FRAME_ID, "op": "lifecycle"}).encode()
        value = json.loads(response(Controller(), raw))
        self.assertFalse(value["ok"])
        self.assertEqual(value["id"], FRAME_ID)
        self.assertEqual(value["error"]["code"], "not_bootstrapped")

    def test_invalid_json_envelope_has_no_identity(self):
        value = json.loads(response(Controller(), b"{broken"))
        self.assertFalse(value["ok"])
        self.assertIsNone(value["id"])
        self.assertEqual(value["error"]["code"], "invalid_request")

    def test_envelope_is_single_canonical_line(self):
        raw = json.dumps({"id": FRAME_ID, "op": "lifecycle"}).encode()
        payload = response(Controller(), raw)
        self.assertTrue(payload.endswith(b"\n"))
        self.assertEqual(payload.count(b"\n"), 1)


if __name__ == "__main__":
    unittest.main()
