import unittest

from tools.verify_resource_course import inspect_attempt


class CourseEvidenceTests(unittest.TestCase):
    def evidence(self):
        server = ('COURSE_VERIFY_STARTED actor=11 epoch=9\n'
                  'COURSE_VERIFY_RESULT kind=completed actor=11 score=175 elapsed_ms=4200 checkpoints=3 pickups=1 contacts=0 rules=course-v1\n')
        client = 'player teleport spawn on_board=true\nCOURSE_VERIFY_RUNNING actor=11\n'
        for z in (0, 1, 2, 3, 4, 5.5):
            client += f'COURSE_VERIFY_NATIVE actor=11 position=-8,0.1,{z} on_board=true state=100\n'
        return server, client + 'COURSE_VERIFY_FINISHED actor=11\n'

    def test_requires_host_result_and_native_continuous_onboard_motion(self):
        server, client = self.evidence()
        self.assertTrue(inspect_attempt(server, client, '11')['ok'])
        for invalid in ('', server.replace('rules=course-v1', 'rules=client-score'),
                        'COURSE_VERIFY_STARTED actor=11 epoch=8\n' + server,
                        server.replace('actor=11 score', 'actor=12 score'),
                        server.replace('score=175', 'score=1'),
                        server.replace('elapsed_ms=4200', 'elapsed_ms=0')):
            self.assertFalse(inspect_attempt(invalid, client, '11')['ok'])
        self.assertFalse(inspect_attempt(server, client.replace('on_board=true state', 'on_board=false state'), '11')['ok'])
        self.assertFalse(inspect_attempt(server, client.replace('5.5', '0'), '11')['ok'])

    def test_rejects_reconciliation_during_attempt_even_with_a_completed_line(self):
        server, client = self.evidence()
        invalid = client.replace('COURSE_VERIFY_NATIVE actor=11 position=-8,0.1,3',
                                 'player teleport spawn on_board=true\nCOURSE_VERIFY_NATIVE actor=11 position=-8,0.1,3')
        self.assertFalse(inspect_attempt(server, invalid, '11')['ok'])
        self.assertFalse(inspect_attempt(server + 'COURSE_VERIFY_RESULT kind=rejected actor=11 reason=speed\n', client, '11')['ok'])

    def test_duplicate_logging_cannot_satisfy_native_sample_minimum(self):
        server, _ = self.evidence()
        client = 'COURSE_VERIFY_RUNNING actor=11\n'
        for z in (0, 3, 5.5):
            observation = f'COURSE_VERIFY_NATIVE actor=11 position=-8,0.1,{z} on_board=true state=100\n'
            client += observation * 2
        client += 'COURSE_VERIFY_FINISHED actor=11\n'
        report = inspect_attempt(server, client, '11')
        self.assertFalse(report['ok'])
        self.assertEqual(len(report['native_samples']), 3)


if __name__ == '__main__':
    unittest.main()
