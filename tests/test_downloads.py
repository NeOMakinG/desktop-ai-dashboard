import sys, os; sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import unittest
from forma_runtime.downloads import DownloadManager


class FakeClock(object):
    def __init__(self, value=1000):
        self.value = value

    def __call__(self):
        return self.value

    def advance(self, delta):
        self.value += delta
        return self.value


class DownloadManagerTests(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock(1000)
        self.manager = DownloadManager(self.clock)

    def test_start_download_record_shape(self):
        record = self.manager.start_download(
            'https://example.com/installer.dmg', 'installer.dmg',
            mime='application/octet-stream', now=1234)
        self.assertEqual(record, {
            'id': 'dl-000001',
            'url': 'https://example.com/installer.dmg',
            'filename': 'installer.dmg',
            'mime': 'application/octet-stream',
            'state': 'active',
            'started_at': 1234,
            'received_bytes': 0,
            'total_bytes': None,
            'finished_at': None,
            'error': None,
        })

    def test_start_download_uses_clock_when_now_omitted(self):
        record = self.manager.start_download('https://example.com/a.zip', 'a.zip')
        self.assertEqual(record['started_at'], 1000)
        self.clock.advance(50)
        second = self.manager.start_download('https://example.com/b.zip', 'b.zip')
        self.assertEqual(second['started_at'], 1050)
        self.assertEqual(second['id'], 'dl-000002')

    def test_active_to_completed_lifecycle(self):
        record = self.manager.start_download('https://example.com/guide.pdf', 'guide.pdf', now=10)
        self.manager.update_progress(record['id'], 500, total_bytes=1000, now=11)
        done = self.manager.complete(record['id'], now=12)
        self.assertEqual(done['state'], 'completed')
        self.assertEqual(done['finished_at'], 12)
        self.assertEqual(done['received_bytes'], 500)
        self.assertEqual(done['total_bytes'], 1000)
        self.assertIsNone(done['error'])

    def test_active_to_failed(self):
        record = self.manager.start_download('https://example.com/movie.mp4', 'movie.mp4', now=20)
        failed = self.manager.fail(record['id'], 'connection reset', now=25)
        self.assertEqual(failed['state'], 'failed')
        self.assertEqual(failed['error'], 'connection reset')
        self.assertEqual(failed['finished_at'], 25)

    def test_active_to_cancelled(self):
        record = self.manager.start_download('https://example.com/album.zip', 'album.zip', now=30)
        cancelled = self.manager.cancel(record['id'], now=33)
        self.assertEqual(cancelled['state'], 'cancelled')
        self.assertEqual(cancelled['finished_at'], 33)
        self.assertIsNone(cancelled['error'])

    def test_cancel_after_complete_rejected(self):
        record = self.manager.start_download('https://example.com/doc.pdf', 'doc.pdf', now=40)
        self.manager.complete(record['id'], now=41)
        with self.assertRaises(ValueError):
            self.manager.cancel(record['id'], now=42)
        with self.assertRaises(ValueError):
            self.manager.fail(record['id'], 'late error', now=43)
        with self.assertRaises(ValueError):
            self.manager.complete(record['id'], now=44)

    def test_progress_clamped_to_total(self):
        record = self.manager.start_download('https://example.com/big.iso', 'big.iso')
        updated = self.manager.update_progress(record['id'], 5000, total_bytes=4000)
        self.assertEqual(updated['total_bytes'], 4000)
        self.assertEqual(updated['received_bytes'], 4000)
        later = self.manager.update_progress(record['id'], 9000)
        self.assertEqual(later['received_bytes'], 4000)

    def test_progress_on_terminal_rejected(self):
        record = self.manager.start_download('https://example.com/x.zip', 'x.zip')
        self.manager.cancel(record['id'])
        with self.assertRaises(ValueError):
            self.manager.update_progress(record['id'], 100)

    def test_unknown_id_raises_key_error(self):
        with self.assertRaises(KeyError):
            self.manager.update_progress('dl-999999', 10)
        with self.assertRaises(KeyError):
            self.manager.complete('dl-999999')
        with self.assertRaises(KeyError):
            self.manager.fail('dl-999999', 'reason')
        with self.assertRaises(KeyError):
            self.manager.cancel('dl-999999')
        with self.assertRaises(KeyError):
            self.manager.remove('dl-999999')

    def test_list_filter_and_newest_first_order(self):
        first = self.manager.start_download('https://example.com/1.bin', '1.bin', now=100)
        second = self.manager.start_download('https://example.com/2.bin', '2.bin', now=200)
        third = self.manager.start_download('https://example.com/3.bin', '3.bin', now=300)
        self.manager.complete(first['id'], now=500)
        self.manager.cancel(third['id'], now=400)
        listed = self.manager.list_downloads()
        self.assertEqual(
            [r['id'] for r in listed],
            [first['id'], third['id'], second['id']])
        active_only = self.manager.list_downloads(state='active')
        self.assertEqual([r['id'] for r in active_only], [second['id']])
        completed_only = self.manager.list_downloads(state='completed')
        self.assertEqual([r['id'] for r in completed_only], [first['id']])
        cancelled_only = self.manager.list_downloads(state='cancelled')
        self.assertEqual([r['id'] for r in cancelled_only], [third['id']])
        self.assertEqual(self.manager.list_downloads(state='failed'), [])

    def test_list_tie_resolved_by_id_ascending(self):
        first = self.manager.start_download('https://example.com/a.bin', 'a.bin', now=100)
        second = self.manager.start_download('https://example.com/b.bin', 'b.bin', now=200)
        third = self.manager.start_download('https://example.com/c.bin', 'c.bin', now=200)
        ordered = self.manager.list_downloads()
        self.assertEqual(
            [r['id'] for r in ordered],
            [second['id'], third['id'], first['id']])

    def test_stats_counts(self):
        a = self.manager.start_download('https://example.com/a.zip', 'a.zip')
        b = self.manager.start_download('https://example.com/b.zip', 'b.zip')
        c = self.manager.start_download('https://example.com/c.zip', 'c.zip')
        self.manager.start_download('https://example.com/d.zip', 'd.zip')
        self.manager.complete(a['id'])
        self.manager.fail(b['id'], 'timeout')
        self.manager.cancel(c['id'])
        self.assertEqual(self.manager.stats(), {
            'active': 1, 'completed': 1, 'failed': 1, 'cancelled': 1})
        self.manager.remove(b['id'])
        self.assertEqual(self.manager.stats(), {
            'active': 1, 'completed': 1, 'failed': 0, 'cancelled': 1})

    def test_remove_rules(self):
        active = self.manager.start_download('https://example.com/live.zip', 'live.zip')
        with self.assertRaises(ValueError):
            self.manager.remove(active['id'])
        self.manager.complete(active['id'])
        self.manager.remove(active['id'])
        with self.assertRaises(KeyError):
            self.manager.remove(active['id'])
        cancelled = self.manager.start_download('https://example.com/gone.zip', 'gone.zip')
        self.manager.cancel(cancelled['id'])
        self.manager.remove(cancelled['id'])
        self.assertEqual(
            [r['id'] for r in self.manager.list_downloads()], [])

    def test_returned_copies_do_not_expose_internal_state(self):
        record = self.manager.start_download('https://example.com/copy.zip', 'copy.zip')
        record['state'] = 'completed'
        record['received_bytes'] = 99999
        listed = self.manager.list_downloads()
        self.assertEqual(listed[0]['state'], 'active')
        self.assertEqual(listed[0]['received_bytes'], 0)
        updated = self.manager.update_progress(record['id'], 10)
        updated['received_bytes'] = 88888
        again = self.manager.update_progress(record['id'], 20)
        self.assertEqual(again['received_bytes'], 20)


if __name__ == '__main__':
    unittest.main()
