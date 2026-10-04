resource.export('preview', function(payload)
    local result = resource.call('js-rules', 'checkpoint_points', payload)
    return {points = result.points, label = 'Checkpoint rule preview'}
end)
