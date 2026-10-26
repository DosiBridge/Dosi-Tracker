namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>What the rest of the server may know about the configured provider: never its key or endpoint.</summary>
/// <param name="DisplayName">Shown to owners in the disclosure (<c>AI:Disclosure:ProviderDisplayName</c>).</param>
/// <param name="Provider">The configured protocol (<c>AI:Provider</c>).</param>
/// <param name="DefaultModel">The model features call unless they configure their own.</param>
/// <param name="SupportsJsonSchema">Strict json_schema output; otherwise the schema goes into the instructions.</param>
/// <param name="SupportsTools">Tool calls, which Ask AI needs.</param>
public sealed record AiProviderDescriptor(
    string DisplayName,
    string Provider,
    string DefaultModel,
    bool SupportsJsonSchema,
    bool SupportsTools);
