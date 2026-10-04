resource.lifecycle({on_ui_update() {
    const local=resource.players().find(player=>player.local);
    const marker=local ? resource.state.get('marker',{kind:'player',id:local.id}) : null;
    sdk.ui.text('creator-park', marker ? 'CREATOR PARK — '+marker.label+' ('+marker.type+')' : 'CREATOR PARK — authored ramps, curved rail and practice checkpoints');
}, on_unload() { sdk.ui.remove('creator-park'); }});
