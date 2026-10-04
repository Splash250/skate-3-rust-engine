// Fixed input demonstrates JS -> Lua -> JS; it does not trust a submitted score.
resource.lifecycle({on_load() {
    const preview = resource.call('lua-rule-adapter', 'preview', {completed: 3});
    resource.state.set('preview', preview);
    const starts = (resource.storage.get('starts') || 0) + 1;
    resource.storage.set('starts', starts);
    resource.state.set('starts', starts);
}});
resource.onNet('ready', (_, sender) => {
    resource.send('greeting', {
        preview: resource.state.get('preview'),
        starts: resource.storage.get('starts')
    }, sender);
});
