namespace Dosi.Tracker.AiAssist.Provider;

/// <summary>Whether this server can call an AI provider, and if not, which part of its configuration is missing.
/// Anything but <see cref="Configured"/> answers every AI request with <c>Tracker:AiNotConfigured</c>.</summary>
public enum AiProviderState
{
    Configured,

    /// <summary><c>AI:Enabled</c> is false (the default).</summary>
    Disabled,

    MissingApiKey,
    MissingModel,

    /// <summary><c>AI:OpenAI:BaseUrl</c> is required but empty, not absolute https, or not in
    /// <c>AI:OpenAI:AllowedHosts</c>.</summary>
    InvalidEndpoint,

    /// <summary><c>AI:Provider</c> names no provider this server has.</summary>
    UnsupportedProvider
}
