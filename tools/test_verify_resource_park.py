import unittest
from tools.verify_resource_park import inspect_capture, assert_statuses
from tools.verify_resource_objects import contact_evidence

class EvidenceTests(unittest.TestCase):
    def test_rejects_successful_exit_after_required_world_silently_retired(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_OK\n'
        native = 'World: Test world generation=2 triangles=12 native_grind_primitives=0\nShared objects: registered:3 live_solids:3\n'
        report = inspect_capture(log, native, 0, True, 'initial')
        self.assertFalse(report['ok'])
        self.assertIn('required park', ' '.join(report['failures']))

    def test_requires_native_and_replication_evidence(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_OK\n'
        native = 'World: Community Practice Park generation=1 triangles=32 native_grind_primitives=1\nShared objects: registered:3 live_solids:3\n'
        self.assertTrue(inspect_capture(log, native, 0, True, 'initial')['ok'])
        self.assertFalse(inspect_capture(log, native, 0, False, 'initial')['ok'])
        self.assertFalse(inspect_capture(log.replace('remote_count:1',''), native, 0, True, 'initial')['ok'])
        self.assertFalse(inspect_capture(log, native.replace('live_solids:3','live_solids:1'), 0, True, 'initial')['ok'])

    def test_intentional_world_stop_has_a_different_final_contract(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_OK\n'
        native = 'World: Test world generation=2 triangles=12 native_grind_primitives=1\nShared objects: registered:3 live_solids:3\n'
        self.assertTrue(inspect_capture(log, native, 0, True, 'world-stop')['ok'])

    def test_periodic_capture_uses_phase_completion_instead_of_final_exit_marker(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_PHASE 2\n'
        native = 'World: Community Practice Park generation=1 triangles=32 native_grind_primitives=1\nShared objects: registered:3 live_solids:3\n'
        self.assertTrue(inspect_capture(log, native, 0, True, 'initial', periodic=True)['ok'])
        self.assertFalse(inspect_capture(log, native, 0, True, 'initial')['ok'])

    def test_latest_console_status_catches_silent_resource_retirement(self):
        with self.assertRaisesRegex(RuntimeError, 'community-park'):
            assert_statuses('community-park: started generation=1\ncommunity-park: stopped generation=2\n', {'community-park': True})
        assert_statuses('community-park: stopped generation=2\n', {'community-park': False})

    def test_empty_private_instance_rejects_a_stale_public_entity_or_peer(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_PHASE 3\n'
        native = ('World: Community Practice Park generation=1 triangles=32 native_grind_primitives=1\n'
                  'Shared objects: registered:0 live_solids:0\n'
                  'Multiplayer: provider:dedicated active:true remote_count:0 rtt_ms:Some(1)\n'
                  'Replicated object inventory: []\nCurrent shared entity contact ids: []\n')
        self.assertTrue(inspect_capture(log,native,0,True,'initial',periodic=True,object_count=0,peer_count=0)['ok'])
        self.assertFalse(inspect_capture(log,native.replace('inventory: []','inventory: [{"id":"2"}]'),0,True,'initial',periodic=True,object_count=0,peer_count=0)['ok'])
        self.assertFalse(inspect_capture(log,native.replace('remote_count:0','remote_count:1'),0,True,'initial',periodic=True,object_count=0,peer_count=0)['ok'])

    def test_presentation_retirement_and_loading_menu_are_native_contracts(self):
        log = 'RESOURCE_ACTIVATED\nMAP_TRANSITION_COMMITTED\nremote_count:1\nGAME_VERIFY_PHASE 3\n'
        native = ('World: Community Practice Park generation=1 triangles=32 native_grind_primitives=1\n'
                  'Shared objects: registered:3 live_solids:3\n'
                  'Resource animation: banks:0 layers:0 appearances:0 attachments:0 pending:0\n'
                  'Gameplay paused: false\nPause menu open: false\n')
        self.assertTrue(inspect_capture(log,native,0,True,'initial',periodic=True,presentation=False,playing=True)['ok'])
        self.assertFalse(inspect_capture(log,native.replace('attachments:0','attachments:1'),0,True,'initial',periodic=True,presentation=False,playing=True)['ok'])
        self.assertFalse(inspect_capture(log,native.replace('menu open: false','menu open: true'),0,True,'initial',periodic=True,presentation=False,playing=True)['ok'])

    def test_crate_interaction_requires_exact_actor_object_contact_and_horizontal_motion(self):
        log = ('RESOURCE_ENTITY_CONTACT entity=2 actor=11 resource=shared-objects\n'
               'OBJECT_VERIFY_SAMPLE actor=11 entity=2 position=2.08,0.6,0\n')
        self.assertTrue(contact_evidence(log,'11','2',[2,.6,0])['ok'])
        self.assertFalse(contact_evidence(log.replace('CONTACT entity=2','CONTACT entity=1'),'11','2',[2,.6,0])['ok'])
        self.assertFalse(contact_evidence(log.replace('CONTACT entity=2 actor=11','CONTACT entity=2 actor=12'),'11','2',[2,.6,0])['ok'])
        self.assertFalse(contact_evidence(log.replace('2.08,0.6,0','2,0.2,0'),'11','2',[2,.6,0])['ok'])

if __name__ == '__main__':
    unittest.main()
