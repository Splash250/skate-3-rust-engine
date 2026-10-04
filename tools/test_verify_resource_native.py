import unittest
import socket
import time
import tempfile
from pathlib import Path
from unittest.mock import patch

from tools.verify_resource_native import inspect_attempt


class NativeEvidenceTests(unittest.TestCase):
    def test_admission_wait_requires_fresh_server_actor_and_instance_proof(self):
        from tools.verify_resource_native import wait_admission
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            old = 'NATIVE_VERIFY_READY actor=7 instance=41 epoch=3\n'
            (run / 'server.log').write_text(old)
            # The client has loaded its resources, but the server has not yet
            # received the delayed readiness acknowledgement.
            (run / 'client0.log').write_text('RESOURCE_ACTIVATED\n')

            def delayed_server_ack(predicate, timeout, description, processes):
                self.assertIsNone(predicate())
                wrong = ('NATIVE_VERIFY_READY actor=8 instance=41 epoch=4\n'
                         'NATIVE_VERIFY_READY actor=7 instance=0 epoch=4\n')
                (run / 'server.log').write_text(old + wrong)
                self.assertIsNone(predicate())
                (run / 'server.log').write_text(old + wrong +
                                              'NATIVE_VERIFY_READY actor=7 instance=41 epoch=5\n')
                self.assertIsNotNone(predicate())

            with patch('tools.verify_resource_native.park.wait_until', side_effect=delayed_server_ack):
                self.assertEqual(wait_admission(run, '7', '41', len(old), []),
                                 {'actor': '7', 'instance': '41', 'epoch': '5'})

    def evidence(self):
        server = ('NATIVE_VERIFY_STARTED actor=7 epoch=19\n'
                  'NATIVE_VERIFY_RESULT actor=7 kind=completed ticks=300 points=22 publications=1 landings=1 '
                  'trick=ID_TRICK_FLIP_HEELFLIP rules=native-input-v1\n')
        client = ('NATIVE_AUTHORITY_ACK epoch=19 tick=296 predicted=300 matched=true\n'
                  'NATIVE_AUTHORITY_COMPLETED epoch=19 tick=300 reconciled=true\n')
        return server, client

    def test_requires_native_semantics_and_matching_terminal_reconciliation(self):
        server, client = self.evidence()
        self.assertTrue(inspect_attempt(server, client, '7')['ok'])
        self.assertFalse(inspect_attempt(server, '', '7')['ok'])
        self.assertFalse(inspect_attempt(server.replace('points=22', 'points=0'), client, '7')['ok'])
        self.assertFalse(inspect_attempt(server.replace('landings=1', 'landings=0'), client, '7')['ok'])
        self.assertFalse(inspect_attempt(server.replace('publications=1', 'publications=0'), client, '7')['ok'])
        self.assertFalse(inspect_attempt(server.replace('native-input-v1', 'course-v1'), client, '7')['ok'])

    def test_rejects_forged_or_replayed_evidence(self):
        server, client = self.evidence()
        self.assertFalse(inspect_attempt(server, client, '8')['ok'])
        self.assertFalse(inspect_attempt(server + server, client, '7')['ok'])
        self.assertFalse(inspect_attempt(server, client.replace('epoch=19', 'epoch=20'), '7')['ok'])
        self.assertFalse(inspect_attempt(server, client + 'Native authority stopped: mismatch', '7')['ok'])


class ImpairedEndpointTests(unittest.TestCase):
    def test_loopback_round_trip_is_delayed_and_cleanup_releases_the_port(self):
        from tools.verify_resource_native import ImpairedEndpoint
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            server.bind(('127.0.0.1', 0)); server.settimeout(2)
            client.bind(('127.0.0.1', 0)); client.settimeout(2)
            endpoint = ImpairedEndpoint(f'127.0.0.1:{server.getsockname()[1]}', 7, delay=(.02, .02), loss=0)
            try:
                started = time.monotonic()
                client.sendto(b'input', endpoint.address)
                data, sender = server.recvfrom(1024)
                self.assertEqual(data, b'input')
                self.assertEqual(sender, endpoint.address)
                server.sendto(b'ack', sender)
                self.assertEqual(client.recvfrom(1024)[0], b'ack')
                self.assertGreaterEqual(time.monotonic() - started, .035)
            finally:
                report = endpoint.close()
            self.assertEqual(report['forwarded'], {'client_to_server': 1, 'server_to_client': 1})
            self.assertEqual(report['queued_datagrams'], 0)
            self.assertFalse(report['thread_alive'])
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as reused:
                reused.bind(endpoint.address)

    def test_configured_loss_does_not_forward_or_retain_payloads(self):
        from tools.verify_resource_native import ImpairedEndpoint
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            server.bind(('127.0.0.1', 0)); server.settimeout(.1)
            endpoint = ImpairedEndpoint(f'127.0.0.1:{server.getsockname()[1]}', 9, loss=1)
            try:
                client.sendto(b'discarded', endpoint.address)
                with self.assertRaises(socket.timeout): server.recvfrom(1024)
            finally:
                report = endpoint.close()
            self.assertEqual(report['random_drops'], 1)
            self.assertEqual(report['queued_bytes'], 0)

    def test_queue_capacity_stays_bounded_and_pending_packets_are_released(self):
        from tools.verify_resource_native import ImpairedEndpoint
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            server.bind(('127.0.0.1', 0)); server.settimeout(.1)
            endpoint = ImpairedEndpoint(f'127.0.0.1:{server.getsockname()[1]}', 11, delay=(1, 1), loss=0)
            endpoint.MAX_DATAGRAMS, endpoint.MAX_BYTES = 2, 16
            try:
                for _ in range(4): client.sendto(b'01234567', endpoint.address)
                with self.assertRaises(socket.timeout): server.recvfrom(1024)
            finally:
                report = endpoint.close()
            self.assertEqual(report['capacity_drops'], 2)
            self.assertEqual(report['peak_queued_datagrams'], 2)
            self.assertEqual(report['peak_queued_bytes'], 16)
            self.assertEqual(report['shutdown_discards'], 2)
            self.assertEqual((report['queued_datagrams'], report['queued_bytes']), (0, 0))
