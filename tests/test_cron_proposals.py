import sys
sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parents[1]))

import unittest
from datetime import datetime, timezone

from runtime.forma_runtime.cron_proposals import (
    ApprovalLedger,
    CronProposal,
    approved_proposals,
)


def make_proposal(**overrides):
    kwargs = dict(
        id="p1",
        cron_expression="0 9 * * 1-5",
        action={"type": "refresh_dashboard", "dashboard": "triage"},
        rationale="weekday morning triage",
    )
    kwargs.update(overrides)
    return CronProposal(**kwargs)


class CronProposalValidationTests(unittest.TestCase):
    def test_rejects_macros(self):
        for expr in ("@daily", "@reboot", "@hourly"):
            with self.assertRaises(ValueError):
                make_proposal(cron_expression=expr)

    def test_rejects_wrong_field_counts(self):
        for expr in ("* * * *", "* * * * * *", "0 9 * * * *", "0 9 * *"):
            with self.assertRaises(ValueError):
                make_proposal(cron_expression=expr)

    def test_rejects_name_tokens(self):
        for expr in ("0 9 * * mon", "0 9 JAN * *"):
            with self.assertRaises(ValueError):
                make_proposal(cron_expression=expr)

    def test_rejects_out_of_range_values(self):
        for expr in ("99 * * * *", "* 25 * * *", "* * 32 * *", "* * * 13 *", "0 9 * * 9"):
            with self.assertRaises(ValueError):
                make_proposal(cron_expression=expr)

    def test_rejects_malformed_ranges_and_steps(self):
        for expr in ("5-1 * * * *", "*/0 * * * *", "1/-2 * * * *", "a * * * *", "*-2 * * * *"):
            with self.assertRaises(ValueError):
                make_proposal(cron_expression=expr)

    def test_accepts_valid_expressions(self):
        for expr in (
            "0 9 * * 1-5",
            "*/5 * * * *",
            "* * * * *",
            "30 2 1 1,7 0",
            "0 0 1-15/3 2 1-5",
            "59 23 31 12 7",
        ):
            self.assertEqual(make_proposal(cron_expression=expr).cron_expression, expr)

    def test_defaults_and_validation(self):
        p = make_proposal()
        self.assertEqual(p.status, "proposed")
        self.assertIsInstance(p.created_at, datetime)
        self.assertIsNotNone(p.created_at.tzinfo)
        with self.assertRaises(ValueError):
            make_proposal(status="bogus")
        with self.assertRaises(ValueError):
            make_proposal(action=["not", "a", "dict"])

    def test_action_dict_is_defensively_copied(self):
        action = {"type": "refresh"}
        p = make_proposal(action=action)
        action["mutated"] = True
        self.assertNotIn("mutated", p.action)


class ApprovalLedgerTests(unittest.TestCase):
    def test_explicit_decision_only(self):
        ledger = ApprovalLedger()
        p = make_proposal()
        ledger.add(p)
        self.assertEqual(p.status, "proposed")
        self.assertIsNone(ledger.decision(p.id))
        before = datetime.now(timezone.utc)
        ledger.decide(p.id, True)
        decision = ledger.decision(p.id)
        self.assertIsNotNone(decision)
        self.assertTrue(decision.approved)
        self.assertIsNotNone(decision.decided_at.tzinfo)
        self.assertGreaterEqual(decision.decided_at, before.replace(tzinfo=decision.decided_at.tzinfo) if before.tzinfo is None else before)
        self.assertEqual(p.status, "approved")

    def test_deny_recorded(self):
        ledger = ApprovalLedger()
        p = make_proposal()
        ledger.add(p)
        ledger.decide(p.id, False)
        decision = ledger.decision(p.id)
        self.assertFalse(decision.approved)
        self.assertEqual(p.status, "denied")

    def test_double_decision_rejected(self):
        ledger = ApprovalLedger()
        ledger.add(make_proposal())
        ledger.decide("p1", True)
        with self.assertRaises(ValueError):
            ledger.decide("p1", False)

    def test_unknown_proposal_rejected(self):
        ledger = ApprovalLedger()
        with self.assertRaises(ValueError):
            ledger.decide("nope", True)

    def test_duplicate_id_rejected(self):
        ledger = ApprovalLedger()
        ledger.add(make_proposal())
        with self.assertRaises(ValueError):
            ledger.add(make_proposal())


class ApprovedEnumerationTests(unittest.TestCase):
    def test_denied_and_proposed_excluded_approved_included(self):
        ledger = ApprovalLedger()
        approved = make_proposal(id="a", cron_expression="0 9 * * 1-5")
        denied = make_proposal(id="b", cron_expression="*/5 * * * *")
        pending = make_proposal(id="c", cron_expression="30 2 * * 0")
        for p in (approved, denied, pending):
            ledger.add(p)
        ledger.decide("a", True)
        ledger.decide("b", False)
        result = approved_proposals(ledger)
        self.assertEqual([p.id for p in result], ["a"])
        self.assertTrue(all(p.status == "approved" for p in result))


if __name__ == "__main__":
    unittest.main()
