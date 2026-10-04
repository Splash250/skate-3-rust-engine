// Original redistributable example. Policy is deliberately resource-owned.
resource.export('checkpoint_points', payload => {
    if (!payload || !Number.isInteger(payload.completed) || payload.completed < 0 || payload.completed > 100) {
        throw Error('completed must be an integer in 0..100');
    }
    return {points: payload.completed * 25};
});
