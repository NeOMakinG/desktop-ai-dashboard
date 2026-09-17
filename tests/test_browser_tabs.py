import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import unittest

from forma_runtime.browser_tabs import (
    AGENT_DRIVING,
    BrowserTabError,
    BrowserTabRegistry,
    IDLE,
    InvalidGrantError,
    InvalidProfileError,
    ProfileRegistry,
    ProfileSwitchConflictError,
    TabClosedError,
    UnknownTabError,
    redact_url,
)


class TestOpenTab(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()

    def test_first_open_fields_and_defaults(self):
        rec = self.registry.open_tab("https://a.example/", "work")
        self.assertEqual(rec.tab_id, "tab-1")
        self.assertEqual(rec.url, "https://a.example/")
        self.assertEqual(rec.profile_id, "work")
        self.assertEqual(rec.automation_state, IDLE)
        self.assertTrue(rec.is_open)
        self.assertIsNone(rec.grant_id)
        self.assertIsNone(rec.grant_expires_at)

    def test_open_rejects_empty_url_or_profile(self):
        with self.assertRaises(BrowserTabError):
            self.registry.open_tab("", "work")
        with self.assertRaises(BrowserTabError):
            self.registry.open_tab("https://a.example/", "")

    def test_first_tab_becomes_active(self):
        rec = self.registry.open_tab("https://a.example/", "work")
        self.assertEqual(self.registry.active_tab_id, rec.tab_id)

    def test_ids_never_reused_after_close(self):
        first = self.registry.open_tab("https://a.example/", "work")
        self.registry.close_tab(first.tab_id)
        second = self.registry.open_tab("https://b.example/", "work")
        self.assertNotEqual(first.tab_id, second.tab_id)


class TestTombstones(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()
        self.tab = self.registry.open_tab("https://a.example/", "work")

    def test_close_leaves_inspectable_record(self):
        closed = self.registry.close_tab(self.tab.tab_id)
        self.assertFalse(closed.is_open)
        same = self.registry.get_tab(self.tab.tab_id)
        self.assertFalse(same.is_open)

    def test_operations_on_closed_raise(self):
        tid = self.tab.tab_id
        self.registry.close_tab(tid)
        with self.assertRaises(TabClosedError):
            self.registry.close_tab(tid)
        with self.assertRaises(TabClosedError):
            self.registry.activate_tab(tid)
        with self.assertRaises(TabClosedError):
            self.registry.set_agent_driving(tid, "grant-1", 900.0)
        with self.assertRaises(TabClosedError):
            self.registry.set_idle(tid)

    def test_unknown_id_raises(self):
        with self.assertRaises(UnknownTabError):
            self.registry.get_tab("tab-999")
        with self.assertRaises(UnknownTabError):
            self.registry.close_tab("tab-999")

    def test_closing_active_clears_active_id(self):
        tid = self.tab.tab_id
        self.registry.close_tab(tid)
        self.assertIsNone(self.registry.active_tab_id)


class TestActivation(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()
        self.first = self.registry.open_tab("https://a.example/", "work")
        self.second = self.registry.open_tab("https://b.example/", "work")

    def test_activate_switches_active(self):
        rec = self.registry.activate_tab(self.first.tab_id)
        self.assertEqual(self.registry.active_tab_id, self.first.tab_id)
        self.assertEqual(rec.tab_id, self.first.tab_id)

    def test_activate_closed_or_unknown_raises(self):
        self.registry.close_tab(self.first.tab_id)
        with self.assertRaises(TabClosedError):
            self.registry.activate_tab(self.first.tab_id)
        with self.assertRaises(UnknownTabError):
            self.registry.activate_tab("tab-999")

    def test_open_tab_ids_sorted_and_filtered(self):
        self.assertEqual(
            self.registry.open_tab_ids(),
            sorted([self.first.tab_id, self.second.tab_id]),
        )
        self.assertEqual(
            self.registry.open_tab_ids(profile_id="work"),
            sorted([self.first.tab_id, self.second.tab_id]),
        )
        self.assertEqual(self.registry.open_tab_ids(profile_id="home"), [])


class TestGrants(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()
        self.tab = self.registry.open_tab("https://a.example/", "work")

    def test_agent_driving_records_grant(self):
        rec = self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        self.assertEqual(rec.automation_state, AGENT_DRIVING)
        self.assertEqual(rec.grant_id, "grant-1")
        self.assertEqual(rec.grant_expires_at, 900.0)

    def test_invalid_grant_inputs_raise(self):
        tid = self.tab.tab_id
        with self.assertRaises(InvalidGrantError):
            self.registry.set_agent_driving(tid, "", 900.0)
        with self.assertRaises(InvalidGrantError):
            self.registry.set_agent_driving(tid, "grant-1", None)

    def test_set_idle_clears_grant(self):
        self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        rec = self.registry.set_idle(self.tab.tab_id)
        self.assertEqual(rec.automation_state, IDLE)
        self.assertIsNone(rec.grant_id)
        self.assertIsNone(rec.grant_expires_at)

    def test_revoke_forces_idle_sorted(self):
        other = self.registry.open_tab("https://b.example/", "work")
        self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        self.registry.set_agent_driving(other.tab_id, "grant-1", 900.0)
        forced = self.registry.revoke_grant("grant-1")
        self.assertEqual(forced, sorted([self.tab.tab_id, other.tab_id]))
        self.assertEqual(self.registry.get_tab(self.tab.tab_id).automation_state, IDLE)
        self.assertIsNone(self.registry.get_tab(other.tab_id).grant_id)

    def test_revoke_only_matching_grant(self):
        other = self.registry.open_tab("https://b.example/", "work")
        self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        self.registry.set_agent_driving(other.tab_id, "grant-2", 900.0)
        forced = self.registry.revoke_grant("grant-2")
        self.assertEqual(forced, [other.tab_id])
        self.assertEqual(
            self.registry.get_tab(self.tab.tab_id).automation_state, AGENT_DRIVING
        )

    def test_rejects_empty_grant_id(self):
        with self.assertRaises(InvalidGrantError):
            self.registry.revoke_grant("")

    def test_refresh_expires_overdue_grants(self):
        self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        forced = self.registry.refresh_automation_status(900.0)
        self.assertEqual(forced, [self.tab.tab_id])
        self.assertEqual(self.registry.get_tab(self.tab.tab_id).automation_state, IDLE)

    def test_refresh_keeps_live_grant(self):
        self.registry.set_agent_driving(self.tab.tab_id, "grant-1", 900.0)
        forced = self.registry.refresh_automation_status(899.0)
        self.assertEqual(forced, [])
        self.assertEqual(
            self.registry.get_tab(self.tab.tab_id).automation_state, AGENT_DRIVING
        )

    def test_refresh_forces_grantless_driving(self):
        record = self.registry.get_tab(self.tab.tab_id)
        record.automation_state = AGENT_DRIVING
        forced = self.registry.refresh_automation_status(1000.0)
        self.assertEqual(forced, [self.tab.tab_id])
        self.assertEqual(self.registry.get_tab(self.tab.tab_id).automation_state, IDLE)

    def test_refresh_skips_closed_and_idle(self):
        self.registry.set_idle(self.tab.tab_id)
        other = self.registry.open_tab("https://b.example/", "work")
        self.registry.close_tab(other.tab_id)
        self.assertEqual(self.registry.refresh_automation_status(1000.0), [])


class TestSummary(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()
        self.first = self.registry.open_tab(
            "https://mail.example/inbox?session=xyz#top", "work"
        )
        self.second = self.registry.open_tab("https://docs.example/", "work")

    def test_summary_redacts_urls(self):
        summary = self.registry.to_redacted_summary()
        by_id = {t["tab_id"]: t for t in summary["tabs"]}
        self.assertEqual(
            by_id[self.first.tab_id]["redacted_url"], "https://mail.example/inbox"
        )
        self.assertNotIn("session", by_id[self.first.tab_id]["redacted_url"])
        self.assertNotIn("top", by_id[self.first.tab_id]["redacted_url"])

    def test_summary_excludes_closed_and_flags_active(self):
        self.registry.set_agent_driving(self.second.tab_id, "grant-1", 900.0)
        summary = self.registry.to_redacted_summary()
        self.assertEqual(summary["agent_driven_tab_ids"], [self.second.tab_id])
        self.registry.activate_tab(self.second.tab_id)
        self.registry.close_tab(self.first.tab_id)
        summary = self.registry.to_redacted_summary()
        listed = [t["tab_id"] for t in summary["tabs"]]
        self.assertEqual(listed, [self.second.tab_id])
        self.assertTrue(summary["tabs"][0]["is_active"])


class TestRedactUrl(unittest.TestCase):
    def test_redact_strips_query_and_fragment(self):
        self.assertEqual(
            redact_url("https://a.example/p?secret=1#frag"), "https://a.example/p"
        )

    def test_redact_keeps_scheme_host_path(self):
        self.assertEqual(
            redact_url("http://b.example:8080/deep/path"), "http://b.example:8080/deep/path"
        )


class TestProfiles(unittest.TestCase):
    def setUp(self):
        self.registry = BrowserTabRegistry()
        self.profiles = ProfileRegistry(self.registry)

    def test_add_requires_absolute_dir(self):
        rec = self.profiles.add_profile("work", "/tmp/forma-profiles/work")
        self.assertEqual(rec.user_data_dir, "/tmp/forma-profiles/work")
        with self.assertRaises(InvalidProfileError):
            self.profiles.add_profile("bad", "relative/path")

    def test_add_rejects_empty_and_duplicates(self):
        with self.assertRaises(InvalidProfileError):
            self.profiles.add_profile("", "/tmp/x")
        with self.assertRaises(InvalidProfileError):
            self.profiles.add_profile("work", "   ")
        self.profiles.add_profile("work", "/tmp/forma-profiles/work")
        with self.assertRaises(InvalidProfileError):
            self.profiles.add_profile("work", "/tmp/forma-profiles/work2")

    def test_get_unknown_raises(self):
        with self.assertRaises(InvalidProfileError):
            self.profiles.get_profile("nope")

    def test_set_active_unknown_raises(self):
        with self.assertRaises(InvalidProfileError):
            self.profiles.set_active_profile("nope")

    def test_switch_conflict_lists_offenders_sorted(self):
        self.profiles.add_profile("work", "/tmp/forma-profiles/work")
        self.profiles.add_profile("home", "/tmp/forma-profiles/home")
        self.profiles.set_active_profile("home")
        late = self.registry.open_tab("https://a.example/", "home")
        early = self.registry.open_tab("https://b.example/", "home")
        try:
            self.profiles.set_active_profile("work")
            self.fail("expected ProfileSwitchConflictError")
        except ProfileSwitchConflictError as err:
            self.assertEqual(
                err.offending_tab_ids, sorted([early.tab_id, late.tab_id])
            )
            self.assertEqual(err.profile_id, "work")

    def test_switch_succeeds_after_conflicts_closed(self):
        self.profiles.add_profile("work", "/tmp/forma-profiles/work")
        self.profiles.add_profile("home", "/tmp/forma-profiles/home")
        self.profiles.set_active_profile("home")
        tab = self.registry.open_tab("https://a.example/", "home")
        with self.assertRaises(ProfileSwitchConflictError):
            self.profiles.set_active_profile("work")
        self.registry.close_tab(tab.tab_id)
        self.assertEqual(self.profiles.set_active_profile("work"), "work")
        self.assertEqual(self.profiles.active_profile_id, "work")

    def test_switch_without_registry_always_ok(self):
        bare = ProfileRegistry()
        bare.add_profile("work", "/tmp/forma-profiles/work")
        self.assertEqual(bare.set_active_profile("work"), "work")

    def test_profile_ids_sorted(self):
        self.profiles.add_profile("zeta", "/tmp/forma-profiles/zeta")
        self.profiles.add_profile("alpha", "/tmp/forma-profiles/alpha")
        self.assertEqual(self.profiles.profile_ids(), ["alpha", "zeta"])


if __name__ == "__main__":
    unittest.main()
