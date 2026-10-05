//! Adapter from admitted native sessions to the transport-independent resource API.
use skate_accounts::VerifiedSession;
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn adapter(
    sessions: BTreeMap<u64, VerifiedSession>,
) -> skate_mods::resources::Authorizer {
    // A caller cannot associate a verified session with a different actor key.
    // Login already caps the entire account authority at 256 live sessions.
    let sessions: BTreeMap<_, _> = sessions
        .into_iter()
        .filter(|(actor, session)| *actor == session.actor() && session.is_active())
        .take(256)
        .collect();
    Arc::new(move |actor, permission| {
        sessions
            .get(&actor)
            .is_some_and(|session| session.permits(permission))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_accounts::AccountStore;
    #[test]
    fn real_sessions_enforce_roles_revocation_connection_and_actor_identity() {
        let root =
            std::env::temp_dir().join(format!("skate-resource-permissions-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let store = AccountStore::open(root.join("accounts.sqlite3")).unwrap();
        store
            .bootstrap_admin("administrator", "test-password-12345")
            .unwrap();
        let admin_login = store.login("administrator", "test-password-12345").unwrap();
        let admin = store.authenticate(&admin_login.token).unwrap();
        let account = store
            .create_account(&admin, "player", "test-password-12345")
            .unwrap();
        let login = store.login("player", "test-password-12345").unwrap();
        let player = store.authenticate(&login.token).unwrap();
        let check = adapter(BTreeMap::from([
            (admin.actor(), admin.clone()),
            (player.actor(), player.clone()),
        ]));
        assert!(check(admin.actor(), "calls.test"));
        assert!(!check(player.actor(), "calls.test"));
        assert!(!check(0, "calls.test"));
        store.create_role(&admin, "diagnostics", None).unwrap();
        store
            .role_permission(&admin, "diagnostics", "calls.test", true)
            .unwrap();
        store
            .assign_role(&admin, &account.id, "diagnostics", true)
            .unwrap();
        assert!(
            check(player.actor(), "calls.test"),
            "grant changes the existing live handle"
        );
        store
            .role_permission(&admin, "diagnostics", "calls.test", false)
            .unwrap();
        assert!(
            !check(player.actor(), "calls.test"),
            "revocation is visible without refreshing the adapter"
        );
        let spoofed = adapter(BTreeMap::from([(player.actor(), admin.clone())]));
        assert!(!spoofed(player.actor(), "calls.test"));
        admin.revoke_connection();
        assert!(
            !check(admin.actor(), "calls.test"),
            "disconnected sessions cannot retain permissions"
        );
        let empty = adapter(BTreeMap::new());
        assert!(!empty(player.actor(), "calls.test"));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
