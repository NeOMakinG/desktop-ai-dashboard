import unittest


class TabSessionsTests(unittest.TestCase):
    def test_equal_timestamps_resolve_by_id_asc(self):
        """Ties on timestamp resolve by id ascending."""
        sessions = [
            {"id": 3, "timestamp": 100, "name": "research"},
            {"id": 1, "timestamp": 100, "name": "morning"},
            {"id": 2, "timestamp": 90, "name": "evening"},
        ]
        ordered = sorted(sessions, key=lambda s: (s["timestamp"], s["id"]))
        self.assertEqual(
            [s["id"] for s in ordered], [2, 1, 3]
        )


if __name__ == "__main__":
    unittest.main()
