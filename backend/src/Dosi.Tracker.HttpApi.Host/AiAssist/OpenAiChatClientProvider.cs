using System;
using System.ClientModel;
using System.Collections.Concurrent;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using OpenAI;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The API host's AI provider, and the only code that uses the OpenAI SDK: OpenAI itself, Azure OpenAI's v1 API or an
/// OpenAI-compatible server, through Chat Completions (whose <c>store</c> default is off).
/// <para>The key (<c>AI:OpenAI:ApiKey</c>, from the environment, the git-ignored <c>appsettings.secrets.json</c> or a
/// secret store) is read once, here, and handed only to the SDK client: it is in no options object, DTO or log. Neither
/// is the endpoint.</para>
/// <para>Retries (408, 429, every 5xx, network errors; at most <c>AI:MaxRetries</c>, honouring short Retry-After waits)
/// are the SDK's, through <see cref="OpenAiRetryPolicy"/>; there is no second retry layer on top.</para>
/// </summary>
public sealed class OpenAiChatClientProvider : IAiChatClientProvider
{
    public const string ApiKeyConfigurationKey = "AI:OpenAI:ApiKey";

    /// <summary>Sent in the User-Agent, so the provider's logs show which application called.</summary>
    public const string UserAgentApplicationId = "dosi-tracker";

    private readonly AiOptions _options;
    private readonly OpenAIClient? _client;
    private readonly ConcurrentDictionary<string, IChatClient> _clientsByModel = new(StringComparer.Ordinal);

    public OpenAiChatClientProvider(
        IOptions<AiOptions> options,
        IConfiguration configuration,
        IHostEnvironment environment,
        ILogger<OpenAiChatClientProvider> logger)
        : this(options.Value, configuration[ApiKeyConfigurationKey], environment.IsDevelopment(), logger, configureClient: null)
    {
    }

    /// <param name="configureClient">Tests replace the transport here, so they never reach a provider.</param>
    internal OpenAiChatClientProvider(
        AiOptions options,
        string? apiKey,
        bool isDevelopment,
        ILogger logger,
        Action<OpenAIClientOptions>? configureClient)
    {
        _options = options;
        apiKey = string.IsNullOrWhiteSpace(apiKey) ? null : apiKey.Trim();

        State = AiProviderConfigValidator.Evaluate(options, apiKey != null, isDevelopment);
        if (State == AiProviderState.Configured)
        {
            Descriptor = AiProviderConfigValidator.Describe(options);
            _client = new OpenAIClient(new ApiKeyCredential(apiKey!), BuildClientOptions(options, configureClient));
        }

        LogState(logger);
    }

    public AiProviderState State { get; }

    public AiProviderDescriptor? Descriptor { get; }

    public IChatClient? GetClient(string featureId)
    {
        var feature = AiFeatureIds.Find(featureId) ??
                      throw new ArgumentException($"Unknown AI feature '{featureId}'.", nameof(featureId));
        if (_client == null)
        {
            return null;
        }

        var model = _options.GetModel(feature)!;
        return _clientsByModel.GetOrAdd(model, m => new OpenAiPolicyChatClient(_client.GetChatClient(m).AsIChatClient()));
    }

    private static OpenAIClientOptions BuildClientOptions(AiOptions options, Action<OpenAIClientOptions>? configureClient)
    {
        var clientOptions = new OpenAIClientOptions
        {
            Endpoint = AiProviderConfigValidator.GetEndpoint(options),
            OrganizationId = NullIfBlank(options.OpenAI.OrganizationId),
            ProjectId = NullIfBlank(options.OpenAI.ProjectId),
            UserAgentApplicationId = UserAgentApplicationId,
            NetworkTimeout = AiProviderConfigValidator.GetNetworkTimeout(options),
            RetryPolicy = new OpenAiRetryPolicy(AiProviderConfigValidator.GetMaxRetries(options))
        };

        configureClient?.Invoke(clientOptions);
        return clientOptions;
    }

    private void LogState(ILogger logger)
    {
        if (State == AiProviderState.Configured)
        {
            logger.LogInformation(
                "AI assist: provider state {AiProviderState}, provider {AiProvider}, default model {AiModel}.",
                State, Descriptor!.DisplayName, Descriptor.DefaultModel);
        }
        else if (State == AiProviderState.Disabled)
        {
            logger.LogInformation("AI assist: provider state {AiProviderState} (AI:Enabled is false).", State);
        }
        else
        {
            // Turned on but unusable: an operator meant to configure it.
            logger.LogWarning(
                "AI assist: provider state {AiProviderState}, so AI assist is off. Check the AI configuration section " +
                "(provider {AiProvider}, default model {AiModel}).",
                State, AiProviderConfigValidator.NormalizeProvider(_options.Provider) ?? "(unsupported)",
                NullIfBlank(_options.OpenAI.Model) ?? "(none)");
        }
    }

    private static string? NullIfBlank(string? value) => string.IsNullOrWhiteSpace(value) ? null : value.Trim();
}
