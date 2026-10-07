resource.lifecycle({on_ui_update() {
    const local = resource.players().find(player => player.local);
    const marker = local ? resource.state.get('marker', {kind:'player', id:local.id}) : null;
    sdk.ui.text('boardwalk-borough', marker && marker.label ? 'BOARDWALK BOROUGH · ' + marker.label : 'BOARDWALK BOROUGH · Plaza, pizza, apartments and skate spots');
}, on_unload() { sdk.ui.remove('boardwalk-borough'); }});
