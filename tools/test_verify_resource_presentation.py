import unittest

from tools import verify_resource_presentation as presentation


class PresentationEvidenceTests(unittest.TestCase):
    def test_requires_both_actors_and_expected_clip_layers(self):
        presentation.assert_animation_counts(
            'Resource animation: banks:1 layers:0 appearances:2 attachments:2 pending:0', 'axis_x')
        presentation.assert_animation_counts(
            'Resource animation: banks:1 layers:2 appearances:2 attachments:2 pending:0', 'pose_a')
        with self.assertRaisesRegex(RuntimeError, 'pose_a'):
            presentation.assert_animation_counts(
                'Resource animation: banks:1 layers:1 appearances:1 attachments:1 pending:0', 'pose_a')

    def test_retirement_rejects_remaining_owned_state(self):
        presentation.assert_animation_counts(
            'Resource animation: banks:0 layers:0 appearances:0 attachments:0 pending:0', 'stopped')
        with self.assertRaisesRegex(RuntimeError, 'stopped'):
            presentation.assert_animation_counts(
                'Resource animation: banks:1 layers:0 appearances:0 attachments:0 pending:0', 'stopped')

    def test_missing_or_pending_native_evidence_does_not_pass(self):
        for native in ['', 'Resource animation: banks:1 layers:2 appearances:2 attachments:2 pending:1']:
            with self.assertRaises(RuntimeError):
                presentation.assert_animation_counts(native, 'pose_b')


if __name__ == '__main__':
    unittest.main()
