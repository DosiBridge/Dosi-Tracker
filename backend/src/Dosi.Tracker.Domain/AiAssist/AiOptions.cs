using System;
using System.Collections.Generic;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI assist, configuration section <c>AI</c> (environment variables use the double-underscore form, for example
/// <c>AI__OpenAI__Model</c>). Off unless <see cref="Enabled"/>, and then only for workspaces whose owner opted in.
/// <para>There is deliberately no API key here: the host reads <c>AI:OpenAI:ApiKey</c> once, from the environment, the
/// git-ignored <c>appsettings.secrets.json</c> or a secret store, and keeps it out of every options object, DTO and
/// log.</para>
/// <para>Values are plain strings where an operator could mistype them (provider, structured output, reasoning effort):
/// an unknown value then turns AI assist off (<c>AiProviderConfigValidator</c>) instead of failing the start-up.</para>
/// </summary>
public class AiOptions
{
    public const string SectionName = "AI";

    public const string ProviderOpenAi = "OpenAI";
    public const string ProviderAzureOpenAi = "AzureOpenAI";
    public const string ProviderOpenAiCompatible = "OpenAICompatible";

    public const string StructuredOutputJsonSchema = "JsonSchema";
    public const string StructuredOutputJsonObject = "JsonObject";

    /// <summary>The hard switch (read at start-up).</summary>
    public bool Enabled { get; set; }

    /// <summary><see cref="ProviderOpenAi"/>, <see cref="ProviderAzureOpenAi"/> or <see cref="ProviderOpenAiCompatible"/>.</summary>
    public string Provider { get; set; } = ProviderOpenAi;

    public AiOpenAiOptions OpenAI { get; set; } = new();

    /// <summary>Timeout of one provider attempt, and the deadline of one AI request.</summary>
    public int RequestTimeoutSeconds { get; set; } = 30;

    /// <summary>Retries of a provider call that failed with 408, 429, a 5xx or a network error.</summary>
    public int MaxRetries { get; set; } = 2;

    /// <summary>Omitted when null, because reasoning models reject it; 0.2 suits non-reasoning models.</summary>
    public float? Temperature { get; set; }

    /// <summary>low / medium / high for reasoning models, or null to send none (required for models without
    /// reasoning, such as gpt-4.1-mini).</summary>
    public string? ReasoningEffort { get; set; }

    /// <summary><see cref="StructuredOutputJsonSchema"/> (strict json_schema), or <see cref="StructuredOutputJsonObject"/>
    /// for servers without it (the schema then goes into the instructions).</summary>
    public string StructuredOutput { get; set; } = StructuredOutputJsonSchema;

    /// <summary>False when the model or server cannot call tools; Ask AI is then off.</summary>
    public bool SupportsTools { get; set; } = true;

    /// <summary>Repair turns after output that fails validation, before the fallback.</summary>
    public int RepairAttempts { get; set; } = 1;

    public AiAssistantOptions Assistant { get; set; } = new();

    public AiConcurrencyOptions Concurrency { get; set; } = new();

    public AiQuotaOptions Quotas { get; set; } = new();

    public AiBudgetOptions Budget { get; set; } = new();

    /// <summary>Prices per model, a list so they can be set from the environment (<c>AI__Pricing__0__Model</c>). A model
    /// without a price has no cost estimate; its tokens are still counted.</summary>
    public List<AiModelPricing> Pricing { get; set; } = new();

    /// <summary>Per-feature switches keyed by <see cref="AiFeature.ConfigKey"/> (case-insensitive). A feature that is
    /// not listed is on with its defaults.</summary>
    public Dictionary<string, AiFeatureOptions> Features { get; set; } = new(StringComparer.OrdinalIgnoreCase);

    public AiDisclosureOptions Disclosure { get; set; } = new();

    public AiUsageOptions Usage { get; set; } = new();

    /// <summary>The configured options of <paramref name="feature"/>, or its defaults.</summary>
    public AiFeatureOptions GetFeature(AiFeature feature)
    {
        return Features.TryGetValue(feature.ConfigKey, out var options) && options != null ? options : new AiFeatureOptions();
    }

    /// <summary>The model a feature calls: its own <c>AI:Features:{ConfigKey}:Model</c>, else <c>AI:OpenAI:Model</c>.</summary>
    public string? GetModel(AiFeature feature)
    {
        var model = GetFeature(feature).Model;
        return string.IsNullOrWhiteSpace(model) ? NullIfBlank(OpenAI.Model) : model.Trim();
    }

    /// <summary>The output-token cap of a feature: configured, else the feature's default.</summary>
    public int GetMaxOutputTokens(AiFeature feature)
    {
        var configured = GetFeature(feature).MaxOutputTokens;
        return configured is > 0 ? configured.Value : feature.DefaultMaxOutputTokens;
    }

    private static string? NullIfBlank(string? value) => string.IsNullOrWhiteSpace(value) ? null : value.Trim();
}

/// <summary><c>AI:OpenAI</c>: the OpenAI-protocol endpoint (OpenAI itself, Azure OpenAI's v1 API, or a compatible
/// server).</summary>
public class AiOpenAiOptions
{
    /// <summary>The default model (for Azure OpenAI, the deployment name). Required.</summary>
    public string Model { get; set; } = "";

    /// <summary>Empty for OpenAI itself; required for Azure OpenAI and compatible servers. Absolute https.</summary>
    public string BaseUrl { get; set; } = "";

    /// <summary>Comma-separated hosts <see cref="BaseUrl"/> may point at.</summary>
    public string AllowedHosts { get; set; } = "api.openai.com";

    public string OrganizationId { get; set; } = "";

    public string ProjectId { get; set; } = "";
}

/// <summary><c>AI:Assistant</c>: Ask AI.</summary>
public class AiAssistantOptions
{
    public bool Enabled { get; set; } = true;
    public int MaxToolRounds { get; set; } = 3;
    public int MaxToolCallsPerTurn { get; set; } = 8;
    public int MaxHistoryTurns { get; set; } = 6;
    public int ConversationTtlMinutes { get; set; } = 60;
    public int TurnTimeoutSeconds { get; set; } = 45;
    public int MaxToolResultChars { get; set; } = 6000;
    public int MaxTurnToolChars { get; set; } = 16000;
    public int MaxOutputTokens { get; set; } = 700;
}

/// <summary><c>AI:Concurrency</c>: provider calls in flight per API instance.</summary>
public class AiConcurrencyOptions
{
    public int Global { get; set; } = 16;
    public int PerWorkspace { get; set; } = 3;
}

/// <summary>One allowance. Null means unlimited.</summary>
public class AiQuotaLimits
{
    public int? MonthlyRequests { get; set; }
    public long? MonthlyTokens { get; set; }
    public int? UserDailyRequests { get; set; }
    public int? UserPerMinute { get; set; }
}

/// <summary><c>AI:Quotas</c>: allowances keyed by plan name (case-insensitive), falling back to <see cref="Default"/>
/// for a workspace without a subscription or with an unknown plan.</summary>
public class AiQuotaOptions
{
    public AiQuotaLimits Default { get; set; } = new()
    {
        MonthlyRequests = 200, MonthlyTokens = 800_000, UserDailyRequests = 20, UserPerMinute = 4
    };

    public Dictionary<string, AiQuotaLimits> Plans { get; set; } = new(StringComparer.OrdinalIgnoreCase)
    {
        ["Free"] = new() { MonthlyRequests = 50, MonthlyTokens = 200_000, UserDailyRequests = 10, UserPerMinute = 3 },
        ["Starter"] = new() { MonthlyRequests = 500, MonthlyTokens = 2_000_000, UserDailyRequests = 30, UserPerMinute = 5 },
        ["Business"] = new() { MonthlyRequests = 3000, MonthlyTokens = 10_000_000, UserDailyRequests = 60, UserPerMinute = 6 }
    };

    /// <summary>The allowance of a plan, or <see cref="Default"/>.</summary>
    public AiQuotaLimits For(string? planName)
    {
        return planName != null && Plans.TryGetValue(planName, out var limits) && limits != null ? limits : Default;
    }
}

/// <summary><c>AI:Budget</c>: the platform-wide monthly ceiling. USD is enforced only for models with a price.</summary>
public class AiBudgetOptions
{
    public long? MonthlyTokenLimit { get; set; } = 20_000_000;
    public decimal? MonthlyUsdLimit { get; set; }
}

/// <summary>One entry of <c>AI:Pricing</c>, in USD per million tokens.</summary>
public class AiModelPricing
{
    public string Model { get; set; } = "";
    public decimal InputPer1M { get; set; }
    public decimal? CachedInputPer1M { get; set; }
    public decimal OutputPer1M { get; set; }
}

/// <summary><c>AI:Features:{ConfigKey}</c>.</summary>
public class AiFeatureOptions
{
    public bool Enabled { get; set; } = true;

    /// <summary>Overrides <c>AI:OpenAI:Model</c> for this feature.</summary>
    public string? Model { get; set; }

    /// <summary>Overrides the feature's default output-token cap.</summary>
    public int? MaxOutputTokens { get; set; }
}

/// <summary><c>AI:Disclosure</c>: what owners are told before they turn AI assist on.</summary>
public class AiDisclosureOptions
{
    public string ProviderDisplayName { get; set; } = "OpenAI";
    public string ProcessingRegion { get; set; } = "";
    public string RetentionNote { get; set; } = "";

    /// <summary>The operator's part of the disclosure version; raising it asks every owner to consent again.</summary>
    public int Version { get; set; }
}

/// <summary><c>AI:Usage</c>: the usage ledger.</summary>
public class AiUsageOptions
{
    public int RetentionDays { get; set; } = 400;
}
