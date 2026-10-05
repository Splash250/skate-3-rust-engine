using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Skate.Managed;

public interface IResourceScript { void Start(Resource resource); }

/// <summary>Capability-checked, synchronous resource API. Values cross as JSON.</summary>
public sealed class Resource
{
    private readonly Dictionary<uint, Func<JsonNode?[], JsonNode?>> callbacks = new();
    private readonly Dictionary<uint, (string Kind, string Name)> registrations = new();
    private uint next;
    private readonly int maxCallbacks;
    private readonly int maxValueBytes;
    public string Id { get; }
    public string Version { get; }
    private readonly HashSet<string> grants;
    public bool HasCapability(string name) => grants.Contains(name);
    public string Side { get; }
    public string Generation { get; }
    internal Resource(JsonObject init)
    {
        var metadata = init["metadata"]!;
        Version = metadata["version"]?.GetValue<string>() ?? "";
        grants = metadata["grants"]?.AsObject().Where(x=>x.Value?.GetValue<bool>()==true).Select(x=>x.Key).ToHashSet() ?? new();
        Id = metadata["id"]!.GetValue<string>(); Side = metadata["side"]!.GetValue<string>();
        Generation = metadata["generation"]!.GetValue<string>();
        maxCallbacks = init["maxCallbacks"]!.GetValue<int>();
        maxValueBytes = init["maxValueBytes"]!.GetValue<int>();
    }
    private JsonNode? Call(string method, params JsonNode?[] args) => Wire.Request(new JsonObject
    { ["op"] = "call", ["method"] = method, ["args"] = new JsonArray(args.Select(x => x?.DeepClone()).ToArray()) });
    private void Register(string kind, string name, Func<JsonNode?[], JsonNode?> callback, string permission = "")
    {
        bool replace = kind == "export" && registrations.Values.Any(x => x == (kind, name));
        if (callbacks.Count >= maxCallbacks + 8 && !replace) throw new InvalidOperationException("C# callback limit reached");
        uint id = checked(++next);
        Wire.Request(new JsonObject { ["op"] = "register", ["kind"] = kind, ["name"] = name,
            ["callback"] = id, ["permission"] = permission });
        if (replace) RemoveCallbacks(kind, name);
        callbacks.Add(id, callback); registrations.Add(id, (kind, name));
    }
    private void RemoveCallbacks(string kind, string name)
    {
        foreach (var key in registrations.Where(x => x.Value == (kind, name)).Select(x => x.Key).ToArray())
        { callbacks.Remove(key); registrations.Remove(key); }
    }
    public void Lifecycle(string name, Action<JsonNode?> handler) => Register("lifecycle", name, args => { handler(args.ElementAtOrDefault(0)); return null; });
    public void On(string name, Action<JsonNode?, string> handler) => Register("on", name, args => { handler(args.ElementAtOrDefault(0), args.ElementAtOrDefault(1)?.GetValue<string>() ?? "0"); return null; });
    public void OnNet(string name, Action<JsonNode?, string> handler) => Register("on_net", name, args => { handler(args.ElementAtOrDefault(0), args.ElementAtOrDefault(1)?.GetValue<string>() ?? "0"); return null; });
    public void Off(string name) { Call("resource.off", JsonValue.Create(name)); RemoveCallbacks("on", name); RemoveCallbacks("on_net", name); }
    public void Export(string name, Func<JsonNode?, JsonNode?> handler) => Register("export", name, args => handler(args.ElementAtOrDefault(0)));
    public void Command(string name, string permission, Action<JsonNode?, string> handler) => Register("command", name, args => { handler(args.ElementAtOrDefault(0), args.ElementAtOrDefault(1)?.GetValue<string>() ?? "0"); return null; }, permission);
    public JsonNode? CallExport(string resource, string name, JsonNode? value = null) => Call("resource.call", JsonValue.Create(resource), JsonValue.Create(name), value);
    public void Emit(string name, JsonNode? value = null) => Call("resource.emit", JsonValue.Create(name), value);
    public void Send(string name, JsonNode? value = null, string? recipient = null, JsonNode? scope = null) => Call("resource.send", JsonValue.Create(name), value, JsonValue.Create(recipient), scope);
    public JsonNode? StateGet(string key, JsonNode? scope = null) => Call("resource.state.get", JsonValue.Create(key), scope);
    public void StateSet(string key, JsonNode? value, JsonNode? scope = null) => Call("resource.state.set", JsonValue.Create(key), value, scope);
    public JsonNode? SettingsGet(string key) => Call("resource.settings.get", JsonValue.Create(key));
    public JsonNode? SettingsAll() => Call("resource.settings.all");
    public JsonNode? StorageGet(string key) => Call("resource.storage.get", JsonValue.Create(key));
    public void StorageSet(string key, JsonNode? value) => Call("resource.storage.set", JsonValue.Create(key), value);
    public JsonNode? Players() => Call("resource.players");
    public bool Authorized(string sender, string permission) => Call("resource.authorized", JsonValue.Create(sender), JsonValue.Create(permission))?.GetValue<bool>() == true;
    public void Teleport(string player, JsonNode destination) => Call("resource.teleport", JsonValue.Create(player), destination);
    public JsonNode? Entities() => Call("resource.entities.all");
    public void Entity(JsonNode command) => Call("resource.entities.command", command);
    public void Voice(JsonNode operation) => Call("resource.voice.submit", operation);
    public void World(JsonNode operation) => Call("resource.world.command", operation);
    public void Competition(JsonNode operation) => Call("resource.competition.submit", operation);
    public void TransferStart(string key,string name,JsonNode? payload,JsonNode? options=null) => Call("resource.transfer.start",JsonValue.Create(key),JsonValue.Create(name),payload,options);
    public void TransferCancel(string key) => Call("resource.transfer.cancel",JsonValue.Create(key));
    public void Animation(JsonNode operation) => Submit(new JsonObject {["kind"]="animation",["version"]=1,["operation"]=operation.DeepClone()});
    public void ServiceSubmit(string key, JsonNode operation, int timeoutMs) => Call("resource.services.submit", JsonValue.Create(key), operation, JsonValue.Create(timeoutMs));
    public void ServiceCancel(string key) => Call("resource.services.cancel", JsonValue.Create(key));
    public void Submit(JsonNode command) => Call("sdk.submit", command);
    public void Log(string message) => Call("sdk.log", JsonValue.Create(message));
    public string? ReadText(string path) => Call("sdk.read_text", JsonValue.Create(path))?.GetValue<string>();
    public void UiText(string key, string text) => Call("sdk.ui.text", JsonValue.Create(key), JsonValue.Create(text));
    public void UiRemove(string key) => Call("sdk.ui.remove", JsonValue.Create(key));
    internal JsonNode? Invoke(uint id, JsonArray args)
    {
        var value = callbacks[id](args.Select(x => x?.DeepClone()).ToArray());
        if (Encoding.UTF8.GetByteCount(value?.ToJsonString() ?? "null") > maxValueBytes) throw new InvalidOperationException("C# result exceeds byte limit");
        return value;
    }
}

internal static class Wire
{
    internal const int MaxFrame = 8 * 1024 * 1024;
    private static readonly Stream Input = new BufferedStream(Console.OpenStandardInput(), 16384);
    private static readonly Stream Output = Console.OpenStandardOutput();
    private static ulong next;
    internal static JsonObject Read()
    {
        using var bytes = new MemoryStream();
        for (int i = 0; i < MaxFrame; ++i)
        {
            int value = Input.ReadByte();
            if (value < 0) throw new EndOfStreamException();
            if (value == 10) return JsonNode.Parse(bytes.ToArray(), documentOptions: new() { MaxDepth = 40 })?.AsObject() ?? throw new InvalidDataException("invalid IPC object");
            bytes.WriteByte((byte)value);
        }
        throw new InvalidDataException("IPC frame too large");
    }
    internal static void Write(JsonObject value)
    {
        var bytes = Encoding.UTF8.GetBytes(value.ToJsonString());
        if (bytes.Length >= MaxFrame) throw new InvalidDataException("IPC frame too large");
        Output.Write(bytes); Output.WriteByte(10); Output.Flush();
    }
    internal static JsonNode? Request(JsonObject value)
    {
        ulong id = checked(++next); value["request"] = id; Write(value);
        var reply = Read();
        if (reply["op"]?.GetValue<string>() != "return" || reply["request"]?.GetValue<ulong>() != id) throw new InvalidDataException("unexpected IPC reply");
        if (reply["ok"]?.GetValue<bool>() != true) throw new InvalidOperationException(reply["error"]?.GetValue<string>() ?? "host error");
        return reply["value"]?.DeepClone();
    }
    private static long? cpuBefore;
    private static long? CpuUs() { try { using var process = System.Diagnostics.Process.GetCurrentProcess(); return process.TotalProcessorTime.Ticks / 10; } catch { return null; } }
    internal static void BeginMeasurement(bool enabled) => cpuBefore = enabled ? CpuUs() : null;
    private static long? CpuDelta() { if (!cpuBefore.HasValue) return null; var after=CpuUs(); return cpuBefore.HasValue && after.HasValue ? Math.Max(0,after.Value-cpuBefore.Value) : null; }
    internal static void Done(JsonNode? value) => Write(new() { ["op"] = "done", ["ok"] = true, ["value"] = value?.DeepClone(), ["workerCpuUs"] = CpuDelta() });
    internal static void Error(Exception error) => Write(new() { ["op"] = "done", ["ok"] = false, ["workerCpuUs"] = CpuDelta(), ["error"] = error.GetBaseException().Message[..Math.Min(error.GetBaseException().Message.Length, 2048)] });
}

internal static class Compiler
{
    // Modern .NET has no in-process trust boundary. This policy is an additional
    // native-loading barrier inside the OS-isolated worker, not a replacement.
    private static readonly HashSet<string> AllowedTypes = new(StringComparer.Ordinal) {
        "System.Object", "System.String", "System.Boolean", "System.Char", "System.Byte", "System.SByte",
        "System.Int16", "System.UInt16", "System.Int32", "System.UInt32", "System.Int64", "System.UInt64",
        "System.Single", "System.Double", "System.Decimal", "System.Void", "System.Array", "System.Math", "System.MathF",
        "System.Nullable<T>", "System.ValueTuple<T1, T2>", "System.Tuple<T1, T2>",
        "System.Action", "System.Action<T>", "System.Action<T1, T2>", "System.Func<TResult>", "System.Func<T, TResult>",
        "System.Exception", "System.InvalidOperationException", "System.ArgumentException",
        "System.Collections.Generic.List<T>", "System.Collections.Generic.Dictionary<TKey, TValue>",
        "System.Collections.Generic.KeyValuePair<TKey, TValue>", "System.Collections.Generic.HashSet<T>",
        "System.Collections.Generic.IEnumerable<T>", "System.Collections.Generic.IEnumerator<T>",
        "System.Collections.IEnumerable", "System.Collections.IEnumerator", "System.IDisposable",
        "System.Text.Json.Nodes.JsonNode", "System.Text.Json.Nodes.JsonObject", "System.Text.Json.Nodes.JsonArray", "System.Text.Json.Nodes.JsonValue",
        "Skate.Managed.Resource", "Skate.Managed.IResourceScript"
    };
    private static bool Allowed(ITypeSymbol? type, IAssemblySymbol own)
    {
        if (type is null) return true;
        if (type is IArrayTypeSymbol array) return Allowed(array.ElementType, own);
        if (type is ITypeParameterSymbol) return true;
        if (type is not INamedTypeSymbol named || named.TypeKind == TypeKind.Error) return false;
        if (SymbolEqualityComparer.Default.Equals(named.ContainingAssembly, own)) return named.TypeArguments.All(x => Allowed(x, own));
        return AllowedTypes.Contains(named.OriginalDefinition.ToDisplayString(new SymbolDisplayFormat(typeQualificationStyle: SymbolDisplayTypeQualificationStyle.NameAndContainingTypesAndNamespaces, genericsOptions: SymbolDisplayGenericsOptions.IncludeTypeParameters))) && named.TypeArguments.All(x => Allowed(x, own));
    }
    internal static IResourceScript Build(JsonArray sources)
    {
        if (sources.Count is < 1 or > 128) throw new InvalidDataException("C# source count exceeds limit");
        var trees = sources.Select(source => CSharpSyntaxTree.ParseText(source!["code"]!.GetValue<string>(),
            new CSharpParseOptions(LanguageVersion.CSharp12), source["name"]!.GetValue<string>())).ToArray();
        var referenceNames = new HashSet<string> { "System.Private.CoreLib.dll", "System.Runtime.dll", "System.Collections.dll", "System.Text.Json.dll", "System.Memory.dll" };
        var references = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!).Split(Path.PathSeparator)
            .Where(path => referenceNames.Contains(Path.GetFileName(path))).Select(path => MetadataReference.CreateFromFile(path)).ToList();
        references.Add(MetadataReference.CreateFromFile(typeof(Resource).Assembly.Location));
        var compilation = CSharpCompilation.Create("Resource", trees, references,
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, optimizationLevel: OptimizationLevel.Release, allowUnsafe: false));
        var errors = compilation.GetDiagnostics().Where(x => x.Severity == DiagnosticSeverity.Error).Take(8).ToArray();
        if (errors.Length != 0) throw new InvalidDataException(string.Join("; ", errors.Select(x => x.ToString())));
        foreach (var tree in trees)
        {
            if (tree.GetRoot().DescendantTokens().Any(token => token.IsKind(SyntaxKind.AsyncKeyword)))
                throw new InvalidDataException("C# async callbacks are unsupported; use completion events");
            var model = compilation.GetSemanticModel(tree);
            foreach (var node in tree.GetRoot().DescendantNodesAndSelf())
            {
                if (node is AttributeSyntax or TypeOfExpressionSyntax or PointerTypeSyntax or FunctionPointerTypeSyntax or UnsafeStatementSyntax or StackAllocArrayCreationExpressionSyntax or AwaitExpressionSyntax or DestructorDeclarationSyntax ||
                    node is MethodDeclarationSyntax method && method.Modifiers.Any(SyntaxKind.ExternKeyword))
                    throw new InvalidDataException("C# source uses prohibited native/reflection/async syntax");
                if (node is not ExpressionSyntax expression || expression is OmittedArraySizeExpressionSyntax) continue;
                var type = model.GetTypeInfo(expression).Type;
                if (!Allowed(type, compilation.Assembly)) throw new InvalidDataException($"C# type is outside resource API: {type}");
                var symbol = model.GetSymbolInfo(expression).Symbol;
                if (symbol is null || symbol is INamespaceSymbol || SymbolEqualityComparer.Default.Equals(symbol.ContainingAssembly, compilation.Assembly)) continue;
                if (symbol.Name is "GetType" or "MemberwiseClone" or "GetTypeCode" || !Allowed(symbol.ContainingType, compilation.Assembly))
                    throw new InvalidDataException($"C# member is outside resource API: {symbol}");
                // Methods inherited by Exception/Delegate must not expose reflection handles.
                if (symbol is IMethodSymbol called && !Allowed(called.ReturnType, compilation.Assembly) ||
                    symbol is IPropertySymbol property && !Allowed(property.Type, compilation.Assembly) ||
                    symbol is IFieldSymbol field && !Allowed(field.Type, compilation.Assembly))
                    throw new InvalidDataException($"C# member exposes prohibited type: {symbol}");
            }
        }
        using var output = new MemoryStream();
        var emitted = compilation.Emit(output);
        if (!emitted.Success) throw new InvalidDataException("C# resource compilation failed");
        NativeIsolation.LockDown();
        var assembly = Assembly.Load(output.ToArray());
        var entries = assembly.GetTypes().Where(type => type.IsClass && !type.IsAbstract && typeof(IResourceScript).IsAssignableFrom(type)).ToArray();
        if (entries.Length != 1) throw new InvalidDataException("C# resource must contain exactly one IResourceScript implementation");
        return (IResourceScript)Activator.CreateInstance(entries[0])!;
    }
}

internal static class NativeIsolation
{
    [StructLayout(LayoutKind.Sequential)] private struct Filter { public ushort Code; public byte Jt, Jf; public uint K; }
    [StructLayout(LayoutKind.Sequential)] private unsafe struct Program { public ushort Len; public Filter* Filters; }
    [DllImport("libc", SetLastError = true)] private static extern int prctl(int op, ulong arg2, ulong arg3, ulong arg4, ulong arg5);
    [DllImport("libc", SetLastError = true)] private static extern unsafe long syscall(long number, uint operation, uint flags, Program* program);
    internal static unsafe void LockDown()
    {
        if (OperatingSystem.IsWindows()) return; // AppContainer/Job boundary is established by the trusted parent.
        if (!OperatingSystem.IsLinux()) throw new PlatformNotSupportedException("C# resource isolation unavailable");
        bool x64 = RuntimeInformation.ProcessArchitecture == Architecture.X64;
        if (!x64 && RuntimeInformation.ProcessArchitecture != Architecture.Arm64) throw new PlatformNotSupportedException("C# seccomp architecture unavailable");
        uint arch = x64 ? 0xc000003eU : 0xc00000b7U;
        uint[] denied = x64 ? [59,322,57,58,101,310,311,319,41,42,43,288,53,49,50,44,165,166,155,272,308,321,323]
                            : [221,281,117,270,271,279,198,203,202,242,199,200,201,206,40,39,41,97,268,280,282];
        var filters = new List<Filter> {
            new() {Code=0x20,K=4}, new() {Code=0x15,Jt=1,K=arch}, new() {Code=0x06,K=0x80000000}, new() {Code=0x20,K=0},
            // x32 has a distinct syscall numbering ABI even on x86_64.
            new() {Code=0x45,Jf=1,K=0x40000000}, new() {Code=0x06,K=0x80000000}
        };
        foreach (uint number in denied) { filters.Add(new() {Code=0x15,Jf=1,K=number}); filters.Add(new() {Code=0x06,K=0x0005000d}); }
        filters.Add(new() {Code=0x06,K=0x7fff0000});
        var array = filters.ToArray();
        fixed (Filter* pointer = array)
        {
            var program = new Program { Len=checked((ushort)array.Length), Filters=pointer };
            if (prctl(38,1,0,0,0) != 0 || syscall(x64 ? 317 : 277,1,1,&program) != 0)
                throw new InvalidOperationException("C# seccomp isolation setup failed");
        }
    }
}

internal static class EntryPoint
{
    private static int Main()
    {
        try
        {
            var init = Wire.Read();
            if (init["op"]?.GetValue<string>() != "init") throw new InvalidDataException("expected init");
            Wire.BeginMeasurement(init["profile"]?.GetValue<bool>() == true);
            var resource = new Resource(init);
            var script = Compiler.Build(init["sources"]!.AsArray());
            script.Start(resource); Wire.Done(null);
            while (true)
            {
                var request = Wire.Read();
                if (request["op"]?.GetValue<string>() != "invoke") throw new InvalidDataException("expected invoke");
                Wire.BeginMeasurement(request["profile"]?.GetValue<bool>() == true);
                try { Wire.Done(resource.Invoke(request["callback"]!.GetValue<uint>(), request["args"]!.AsArray())); }
                catch (Exception error) { Wire.Error(error); }
            }
        }
        catch (EndOfStreamException) { return 0; }
        catch (Exception error) { try { Wire.Error(error); } catch { } return 1; }
    }
}
