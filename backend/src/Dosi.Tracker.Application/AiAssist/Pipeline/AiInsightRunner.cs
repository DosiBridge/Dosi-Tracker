using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Localization;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Localization;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// Runs one AI card request (spec 9.1), in this order:
/// <list type="number">
/// <item>the caller's context, then <see cref="AiAccessGuard"/> (provider, feature switch, platform switch,
/// subscription, workspace consent, role);</item>
/// <item>the feature's own authorization, which refuses and never narrows;</item>
/// <item>the per-minute limit;</item>
/// <item>the bundle, read as the caller; an empty one is answered "no data" without calling anyone;</item>
/// <item>the result cache (unless a regenerate within its daily limit): a hit is served even when an allowance is used
/// up;</item>
/// <item>the allowances, the member's in-flight lock, a concurrency slot and the circuit breaker;</item>
/// <item>the provider call under one deadline (<c>AI:RequestTimeoutSeconds</c>, linked to the caller's request), then
/// parsing and validation with at most <c>AI:RepairAttempts</c> repair turns;</item>
/// <item>the ledger row (its own unit of work), the cache, and the rendered DTO.</item>
/// </list>
/// A provider failure, the deadline or an open breaker is answered with the feature's fallback, or
/// <c>Tracker:AiProviderUnavailable</c> without one; output that stays invalid with the fallback, or
/// <c>Tracker:AiOutputInvalid</c>. A caller who goes away gets an <see cref="OperationCanceledException"/> and the row
/// says cancelled. Refusals write no row. Nothing of the prompt, the answer or the provider's text is ever logged.
/// <para>Callers must not hold a database transaction (mark the app-service method
/// <c>[UnitOfWork(isTransactional: false)]</c>): the provider call can take many seconds.</para>
/// </summary>
public class AiInsightRunner : ITransientDependency
{
    public const string DisclaimerKey = "AiAssist:Disclaimer";

    private const int MaxRepairAttempts = 3;

    private static readonly JsonSerializerOptions RawJsonOptions = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    private readonly AiCallerContextFactory _callerContextFactory;
    private readonly AiAccessGuard _accessGuard;
    private readonly AiUsageGate _usageGate;
    private readonly AiResultCache _resultCache;
    private readonly AiUserLock _userLock;
    private readonly AiConcurrencyGate _concurrencyGate;
    private readonly AiCircuitBreaker _circuitBreaker;
    private readonly AiUsageRecorder _recorder;
    private readonly AiCostCalculator _costCalculator;
    private readonly AiMetrics _metrics;
    private readonly AiPromptBuilder _promptBuilder;
    private readonly IAiChatClientProvider _provider;
    private readonly IOptions<AiOptions> _options;
    private readonly ICancellationTokenProvider _cancellationTokenProvider;
    private readonly IUnitOfWorkManager _unitOfWorkManager;
    private readonly IClock _clock;
    private readonly IStringLocalizer<TrackerResource> _localizer;
    private readonly ILogger<AiInsightRunner> _logger;

    public AiInsightRunner(
        AiCallerContextFactory callerContextFactory,
        AiAccessGuard accessGuard,
        AiUsageGate usageGate,
        AiResultCache resultCache,
        AiUserLock userLock,
        AiConcurrencyGate concurrencyGate,
        AiCircuitBreaker circuitBreaker,
        AiUsageRecorder recorder,
        AiCostCalculator costCalculator,
        AiMetrics metrics,
        AiPromptBuilder promptBuilder,
        IAiChatClientProvider provider,
        IOptions<AiOptions> options,
        ICancellationTokenProvider cancellationTokenProvider,
        IUnitOfWorkManager unitOfWorkManager,
        IClock clock,
        IStringLocalizer<TrackerResource> localizer,
        ILogger<AiInsightRunner> logger)
    {
        _callerContextFactory = callerContextFactory;
        _accessGuard = accessGuard;
        _usageGate = usageGate;
        _resultCache = resultCache;
        _userLock = userLock;
        _concurrencyGate = concurrencyGate;
        _circuitBreaker = circuitBreaker;
        _recorder = recorder;
        _costCalculator = costCalculator;
        _metrics = metrics;
        _promptBuilder = promptBuilder;
        _provider = provider;
        _options = options;
        _cancellationTokenProvider = cancellationTokenProvider;
        _unitOfWorkManager = unitOfWorkManager;
        _clock = clock;
        _localizer = localizer;
        _logger = logger;
    }

    public virtual async Task<TOut> RunAsync<TIn, TRaw, TOut>(IAiInsightFeature<TIn, TRaw, TOut> feature, TIn input)
        where TRaw : class
    {
        Check.NotNull(feature, nameof(feature));
        Check.NotNull(input, nameof(input));
        var featureId = feature.FeatureId;
        var featureInfo = AiFeatureIds.Find(featureId)
                          ?? throw new ArgumentException($"'{featureId}' is not an AI assist feature.", nameof(feature));
        var options = _options.Value;

        // 1-2. Who asks, and may they use this feature at all.
        var caller = await _callerContextFactory.CreateAsync(feature.TimeZoneOf(input));
        await _accessGuard.EnsureAsync(caller, featureId);

        // 3. What they may ask it about: refused, never narrowed.
        var scope = await feature.AuthorizeAsync(caller, input);

        // 4. The per-minute limit (every request counts, cache hits included).
        await _usageGate.EnsureBurstAsync(caller, featureId);

        // 5. What the model may be told, read as the caller.
        var bundle = await feature.BuildBundleAsync(caller, input, scope);
        _metrics.InjectionFlags(bundle.InjectionRemovals);
        var request = new RunContext(caller, featureId, feature.PromptVersion, scope, ProviderName(options),
            feature.DataCategories(bundle));

        // 6. Nothing to describe: a fixed answer, nothing sent.
        if (bundle.IsEmpty)
        {
            var noDataId = await RecordAsync(request, AiUsageOutcomes.NoData, model: null, sent: false);
            return feature.NoData(bundle, Meta(request, noDataId, AiResultStatuses.NoData, _clock.Now, model: null, sent: false));
        }

        // 7. The result cache, unless the member asked for a fresh answer and may still have one today.
        var model = options.GetModel(featureInfo) ?? _provider.Descriptor?.DefaultModel ?? "default";
        var cacheKey = AiResultCache.BuildKey(feature.PromptVersion, caller.WorkspaceId, caller.Policy, scope, caller.TimeZone, model, bundle);
        var regenerate = feature.IsRegenerate(input) &&
                         await _usageGate.RegenerateAllowedAsync(caller, featureId, scope.SubjectUserId);
        if (!regenerate)
        {
            var cached = await TryCachedAsync(feature, request, bundle, cacheKey);
            if (cached.Found)
            {
                return cached.Result!;
            }
        }

        // 8. The allowances, counting this request's estimate.
        var jsonObjectMode = string.Equals(options.StructuredOutput, AiOptions.StructuredOutputJsonObject, StringComparison.OrdinalIgnoreCase) ||
                             _provider.Descriptor is { SupportsJsonSchema: false };
        var messages = _promptBuilder.Build(feature.Prompt, bundle, scope.Audience, includeSchema: jsonObjectMode);
        var chatOptions = BuildChatOptions(feature.Prompt, featureInfo, options, jsonObjectMode);
        await _usageGate.EnsureQuotaAsync(caller, featureId, EstimateTokens(messages, chatOptions.MaxOutputTokens ?? 0));

        // 9. One request in flight per member, a slot on this instance, and a provider that is not paused.
        EnsureNoTransaction(_unitOfWorkManager.Current);
        var callerToken = _cancellationTokenProvider.Token;
        await using var userLock = await _userLock.AcquireAsync(caller, featureId, callerToken);
        using var slot = await _concurrencyGate.AcquireAsync(caller.WorkspaceId, featureId, callerToken);

        // 10-11. The call, the repair turn, the fallback.
        var attempt = await CallAsync(feature, request with { IsRegenerate = regenerate }, bundle, messages, chatOptions, model, callerToken);
        request = attempt.Request;

        if (attempt.Raw != null)
        {
            // 12. The ledger, then the cache (pseudonymized raw output), then 13. the rendered answer.
            var resultId = await RecordAsync(request, AiUsageOutcomes.Generated, attempt.Model, sent: true, attempt);
            var generatedAt = _clock.Now;
            await _resultCache.SetAsync(cacheKey, new AiCachedResultItem
            {
                RawJson = JsonSerializer.Serialize(attempt.Raw, RawJsonOptions),
                Model = attempt.Model,
                PromptVersion = feature.PromptVersion,
                InputTokens = attempt.Usage.InputTokens,
                OutputTokens = attempt.Usage.OutputTokens,
                GeneratedAt = generatedAt
            }, feature.CacheTtl(input, bundle));

            return feature.Render(attempt.Raw, bundle,
                Meta(request, resultId, AiResultStatuses.Generated, generatedAt, attempt.Model, sent: true));
        }

        var errorCode = attempt.Failure == null ? TrackerDomainErrorCodes.AiOutputInvalid : TrackerDomainErrorCodes.AiProviderUnavailable;
        if (feature.HasFallback)
        {
            var fallbackId = await RecordAsync(request, AiUsageOutcomes.Fallback, attempt.Model, attempt.Sent, attempt, errorCode);
            return feature.Render(feature.Fallback(bundle), bundle,
                Meta(request, fallbackId, AiResultStatuses.Fallback, _clock.Now, model: null, attempt.Sent));
        }

        // A request the breaker kept from the provider is a refusal: like every refusal, it writes no row.
        if (attempt.Sent)
        {
            var outcome = attempt.Failure?.Kind == AiProviderErrorKind.Timeout ? AiUsageOutcomes.Timeout : AiUsageOutcomes.ProviderError;
            await RecordAsync(request, outcome, attempt.Model, sent: true, attempt, errorCode);
        }

        throw attempt.Failure == null
            ? AiErrors.OutputInvalid(featureId)
            : AiErrors.ProviderUnavailable(attempt.Failure.UnavailableKind, attempt.Failure.RetryAfterSeconds, featureId);
    }

    /// <summary>Calls the provider (and repairs once per <c>AI:RepairAttempts</c>) under one deadline.</summary>
    protected virtual async Task<CallAttempt<TRaw>> CallAsync<TIn, TRaw, TOut>(
        IAiInsightFeature<TIn, TRaw, TOut> feature,
        RunContext request,
        AiContextBundle bundle,
        IList<ChatMessage> messages,
        ChatOptions chatOptions,
        string model,
        CancellationToken callerToken)
        where TRaw : class
    {
        var options = _options.Value;
        var client = _provider.GetClient(request.FeatureId)
                     ?? throw new BusinessException(TrackerDomainErrorCodes.AiNotConfigured).WithData("featureId", request.FeatureId);

        var attempt = new CallAttempt<TRaw>(request) { Model = model };
        var stopwatch = new Stopwatch();
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(callerToken);
        deadline.CancelAfter(TimeSpan.FromSeconds(Math.Clamp(options.RequestTimeoutSeconds, 1, 300)));

        try
        {
            _circuitBreaker.ThrowIfOpen();
            attempt.Sent = true;

            var repairsLeft = Math.Clamp(options.RepairAttempts, 0, MaxRepairAttempts);
            var conversation = messages;
            while (true)
            {
                stopwatch.Start();
                ChatResponse response;
                try
                {
                    response = await GetResponseAsync(client, conversation, chatOptions, request.FeatureId, deadline.Token);
                }
                finally
                {
                    stopwatch.Stop();
                    attempt.LatencyMs = (int)Math.Min(int.MaxValue, stopwatch.ElapsedMilliseconds);
                }

                _circuitBreaker.RecordSuccess();
                attempt.Usage = attempt.Usage.Add(response.Usage);
                attempt.Model = string.IsNullOrWhiteSpace(response.ModelId) ? attempt.Model : response.ModelId;

                var parsed = AiOutputParser.Parse<TRaw>(response);
                var violations = parsed.Succeeded ? feature.Validate(parsed.Value!, bundle) : parsed.Violations;
                if (violations.Count == 0)
                {
                    attempt.Raw = parsed.Value;
                    return attempt;
                }

                // Only the rule messages and their number are logged: they never quote the answer.
                _logger.LogInformation(
                    "AI answer for {FeatureId} broke {ViolationCount} output rules: {Violations}",
                    request.FeatureId, violations.Count, string.Join("; ", violations.Take(5).Select(v => v.Message)));

                if (repairsLeft-- <= 0)
                {
                    return attempt;
                }

                conversation = AiPromptBuilder.BuildRepair(conversation, parsed.Text, violations);
            }
        }
        catch (AiCircuitOpenException open)
        {
            attempt.Failure = new ProviderFailure(null, AiErrors.CircuitOpenKind, open.RetryAfterSeconds);
            return attempt;
        }
        catch (OperationCanceledException) when (callerToken.IsCancellationRequested)
        {
            _circuitBreaker.RecordAbandoned();
            await RecordAsync(attempt.Request, AiUsageOutcomes.Cancelled, attempt.Model, attempt.Sent, attempt);
            throw;
        }
        catch (OperationCanceledException)
        {
            // The server's own deadline, not the caller.
            _circuitBreaker.RecordFailure(AiProviderErrorKind.Timeout);
            attempt.Failure = new ProviderFailure(AiProviderErrorKind.Timeout, AiProviderErrorMapper.ToUnavailableKind(AiProviderErrorKind.Timeout), null);
            return attempt;
        }
        catch (AiProviderException failure)
        {
            _circuitBreaker.RecordFailure(failure.Kind);
            attempt.Failure = new ProviderFailure(failure.Kind, AiProviderErrorMapper.ToUnavailableKind(failure.Kind), failure.RetryAfterSeconds);
            return attempt;
        }
    }

    /// <summary>The provider call. Anything a provider throws besides <see cref="AiProviderException"/> and a cancellation
    /// becomes a server error, logged by type only: its message may quote the provider.</summary>
    private async Task<ChatResponse> GetResponseAsync(
        IChatClient client, IList<ChatMessage> messages, ChatOptions chatOptions, string featureId, CancellationToken token)
    {
        try
        {
            return await client.GetResponseAsync(messages, chatOptions, token);
        }
        catch (Exception e) when (e is not AiProviderException and not OperationCanceledException)
        {
            _logger.LogWarning("AI provider call for {FeatureId} failed with {ExceptionType}.", featureId, e.GetType().Name);
            throw new AiProviderException(AiProviderErrorKind.ServerError);
        }
    }

    private async Task<(bool Found, TOut? Result)> TryCachedAsync<TIn, TRaw, TOut>(
        IAiInsightFeature<TIn, TRaw, TOut> feature, RunContext request, AiContextBundle bundle, string cacheKey)
        where TRaw : class
    {
        var item = await _resultCache.GetAsync(cacheKey);
        if (item == null)
        {
            return (false, default);
        }

        var parsed = AiOutputParser.Parse<TRaw>(item.RawJson, ChatFinishReason.Stop);
        if (!parsed.Succeeded)
        {
            // Written by an older shape of the answer: generate a new one.
            return (false, default);
        }

        var resultId = await RecordAsync(request with { CacheHit = true }, AiUsageOutcomes.Cached, item.Model, sent: true);
        return (true, feature.Render(parsed.Value!, bundle,
            Meta(request, resultId, AiResultStatuses.Cached, item.GeneratedAt, item.Model, sent: true)));
    }

    private async Task<Guid> RecordAsync(
        RunContext request, string outcome, string? model, bool sent, ICallUsage? usage = null, string? errorCode = null)
    {
        var tokens = usage?.Usage ?? AiTokenUsage.None;
        var cost = tokens.IsEmpty
            ? null
            : _costCalculator.Estimate(model, tokens.InputTokens, tokens.CachedInputTokens, tokens.OutputTokens);

        var id = await _recorder.RecordAsync(new AiUsageEntry(
            request.Caller.WorkspaceId,
            request.Caller.UserId,
            request.FeatureId,
            request.PromptVersion,
            request.Scope.Audience,
            request.Provider,
            model,
            outcome)
        {
            SubjectUserId = request.Scope.SubjectUserId,
            ProjectId = request.Scope.ProjectId,
            RangeFrom = request.Scope.RangeFrom,
            RangeTo = request.Scope.RangeTo,
            DataCategories = sent ? request.DataCategories : Array.Empty<string>(),
            CacheHit = request.CacheHit,
            IsRegenerate = request.IsRegenerate,
            ErrorCode = errorCode,
            Usage = tokens,
            EstimatedCostUsd = cost,
            LatencyMs = usage?.LatencyMs
        });

        _metrics.Request(request.FeatureId, outcome);
        _metrics.Tokens(tokens.InputTokens, tokens.OutputTokens);
        if (usage?.LatencyMs is { } latency)
        {
            _metrics.Latency(request.FeatureId, latency);
        }

        _logger.LogInformation(
            "AI request {FeatureId} in workspace {WorkspaceId} by {UserId}: {Outcome} in {LatencyMs} ms, " +
            "{InputTokens} input and {OutputTokens} output tokens, cache hit {CacheHit}, prompt {PromptVersion}.",
            request.FeatureId, request.Caller.WorkspaceId, request.Caller.UserId, outcome, usage?.LatencyMs,
            tokens.InputTokens, tokens.OutputTokens, request.CacheHit, request.PromptVersion);

        return id;
    }

    private AiResultMetaDto Meta(RunContext request, Guid resultId, string status, DateTime generatedAt, string? model, bool sent)
    {
        return new AiResultMetaDto
        {
            ResultId = resultId,
            FeatureId = request.FeatureId,
            Status = status,
            GeneratedAt = generatedAt,
            Model = model,
            PromptVersion = request.PromptVersion,
            DataCategories = sent ? request.DataCategories.ToList() : new List<string>(),
            Disclaimer = _localizer[DisclaimerKey].Value
        };
    }

    /// <summary>The options of one card call: the feature's output limit, the operator's temperature and reasoning
    /// effort when set, and the answer schema (strict json_schema, or "a JSON object" with the schema in the prompt).</summary>
    public static ChatOptions BuildChatOptions(AiPromptSpec prompt, AiFeature feature, AiOptions options, bool jsonObjectMode)
    {
        var configured = options.GetFeature(feature).MaxOutputTokens;
        return new ChatOptions
        {
            MaxOutputTokens = configured is > 0 ? configured.Value : prompt.MaxOutputTokens,
            Temperature = options.Temperature,
            Reasoning = MapReasoning(options.ReasoningEffort),
            ResponseFormat = jsonObjectMode
                ? ChatResponseFormat.Json
                : ChatResponseFormat.ForJsonSchema(prompt.Schema, prompt.SchemaName, prompt.Task)
        };
    }

    /// <summary>low / medium / high (any case) as reasoning options; anything else sends none.</summary>
    public static ReasoningOptions? MapReasoning(string? effort)
    {
        var value = effort?.Trim().ToLowerInvariant();
        return value switch
        {
            "low" => new ReasoningOptions { Effort = ReasoningEffort.Low },
            "medium" => new ReasoningOptions { Effort = ReasoningEffort.Medium },
            "high" => new ReasoningOptions { Effort = ReasoningEffort.High },
            _ => null
        };
    }

    /// <summary>The allowance a card call is charged against before it is made: prompt characters / 4 plus the output
    /// limit.</summary>
    public static long EstimateTokens(IEnumerable<ChatMessage> messages, int maxOutputTokens)
    {
        var characters = messages.Sum(m => (long)(m.Text?.Length ?? 0));
        return characters / 4 + Math.Max(0, maxOutputTokens);
    }

    private string ProviderName(AiOptions options)
    {
        var name = _provider.Descriptor?.Provider;
        return string.IsNullOrWhiteSpace(name) ? options.Provider : name;
    }

    /// <summary>Refuses to call a provider while <paramref name="current"/> holds a database transaction open.</summary>
    /// <exception cref="InvalidOperationException">The unit of work is transactional.</exception>
    public static void EnsureNoTransaction(IUnitOfWork? current)
    {
        if (current is { Options.IsTransactional: true })
        {
            throw new InvalidOperationException(
                "An AI request must not run inside a database transaction; mark the app-service method [UnitOfWork(isTransactional: false)].");
        }
    }

    /// <summary>What one request is, for its ledger row.</summary>
    protected sealed record RunContext(
        AiCallerContext Caller,
        string FeatureId,
        string PromptVersion,
        AiAudienceScope Scope,
        string Provider,
        IReadOnlyList<string> DataCategories)
    {
        public bool CacheHit { get; init; }
        public bool IsRegenerate { get; init; }
    }

    protected interface ICallUsage
    {
        AiTokenUsage Usage { get; }
        int? LatencyMs { get; }
    }

    /// <summary>Why the provider gave no answer.</summary>
    /// <param name="Kind">Null when the breaker kept the call from being made.</param>
    /// <param name="UnavailableKind">The <c>data.kind</c> of <c>Tracker:AiProviderUnavailable</c>.</param>
    protected sealed record ProviderFailure(AiProviderErrorKind? Kind, string UnavailableKind, int? RetryAfterSeconds);

    /// <summary>How the provider call went: a valid answer, a failure, or neither (output that stayed invalid).</summary>
    protected sealed class CallAttempt<TRaw> : ICallUsage
        where TRaw : class
    {
        public CallAttempt(RunContext request)
        {
            Request = request;
        }

        public RunContext Request { get; }
        public TRaw? Raw { get; set; }
        public ProviderFailure? Failure { get; set; }
        public string? Model { get; set; }
        public AiTokenUsage Usage { get; set; } = AiTokenUsage.None;
        public int? LatencyMs { get; set; }

        /// <summary>The provider was called (not kept from it by the breaker).</summary>
        public bool Sent { get; set; }
    }
}
