resource.onNet('greeting', (payload, sender) => {
    if (sender !== '0') return;
    sdk.ui.text('cross-language', `${payload.preview.label}: ${payload.preview.points} | server starts: ${payload.starts}`);
});
resource.lifecycle({
    on_load() { resource.send('ready', {}); },
    on_unload() { sdk.ui.remove('cross-language'); }
});
