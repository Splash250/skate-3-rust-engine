using Skate.Managed;

public sealed class ManagedClient : IResourceScript
{
    public void Start(Resource resource)
    {
        resource.OnNet("greeting", (payload, sender) => resource.UiText("managed-status",
            "C# → Lua → JavaScript preview: " + payload!["preview"]!.ToJsonString()));
        resource.Lifecycle("on_load", payload => resource.Send("ready"));
        resource.Lifecycle("on_unload", payload => resource.UiRemove("managed-status"));
    }
}
