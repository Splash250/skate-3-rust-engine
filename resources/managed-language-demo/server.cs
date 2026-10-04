using Skate.Managed;
using System.Text.Json.Nodes;

public sealed class ManagedServer : IResourceScript
{
    public void Start(Resource resource)
    {
        resource.Lifecycle("on_load", payload => {
            // Fixed server input demonstrates C# -> Lua -> JavaScript.
            var preview = resource.CallExport("lua-rule-adapter", "preview",
                new JsonObject { ["completed"] = 3 });
            resource.StateSet("preview", preview);
            int starts = (resource.StorageGet("starts")?.GetValue<int>() ?? 0) + 1;
            resource.StorageSet("starts", JsonValue.Create(starts));
        });
        resource.OnNet("ready", (payload, sender) => resource.Send("greeting",
            new JsonObject {
                ["preview"] = resource.StateGet("preview"),
                ["starts"] = resource.StorageGet("starts")
            }, sender));
    }
}
