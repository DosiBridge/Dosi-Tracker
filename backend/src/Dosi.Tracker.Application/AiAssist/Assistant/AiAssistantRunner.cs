using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Features;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.AiAssist.Read;
using Microsoft.Extensions.AI;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Threading;
using Volo.Abp.Timing;
using Volo.Abp.Uow;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Runs one Ask AI turn (spec 9.2): the caller's context and the access guard, the conversation (new or loaded, with
/// its consent stamp checked), the question (sanitized, names resolved to handles, or a deterministic clarification),
/// the page context, the burst and quota limits, the lock and concurrency slot and breaker, a tool-calling chat
/// client bound to the turn, validation of the plain-text answer with one repair turn and a facts-list fallback, and
/// finally the ledger row and the stored turn.
/// </summary>
public class AiAssistantRunner : ITransientDependency
{
    private readonly AiCallerContextFactory _callerContextFactory;
    private readonly AiAccessGuard _accessGuard;
    private readonly AiUsageGate _usageGate;
    private readonly AiUserLock _userLock;
    private readonly AiConcurrencyGate _concurrencyGate;
    private readonly AiCircuitBreaker _circuitBreaker;
    private readonly AiUsageRecorder _recorder;
    private readonly AiCostCalculator _costCalculator;
    private readonly AiMetrics _metrics;
    private readonly AiConversationStore _conversationStore;
    private readonly AiToolCatalog _toolCatalog;
    private readonly IAiReadLayer _read;
    private readonly IAiChatClientProvider _provider;
    private readonly IOptions<AiOptions> _options;
    private readonly ICancellationTokenProvider _cancellationTokenProvider;
    private readonly IUnitOfWorkManager _unitOfWorkManager;
    private readonly IClock _clock;
    private readonly ILogger<AiAssistantRunner> _logger;

    public AiAssistantRunner(
        AiCallerContextFactory callerContextFactory,
        AiAccessGuard accessGuard,
        AiUsageGate usageGate,
        AiUserLock userLock,
        AiConcurrencyGate concurrencyGate,
        AiCircuitBreaker circuitBreaker,
        AiUsageRecorder recorder,
        AiCostCalculator costCalculator,
        AiMetrics metrics,
        AiConversationStore conversationStore,
        AiToolCatalog toolCatalog,
        IAiReadLayer read,
        IAiChatClientProvider provider,
        IOptions<AiOptions> options,
        ICancellationTokenProvider cancellationTokenProvider,
        IUnitOfWorkManager unitOfWorkManager,
        IClock clock,
        ILogger<AiAssistantRunner> logger)
    {
        _callerContextFactory = callerContextFactory;
        _accessGuard = accessGuard;
        _usageGate = usageGate;
        _userLock = userLock;
        _concurrencyGate = concurrencyGate;
        _circuitBreaker = circuitBreaker;
        _recorder = recorder;
        _costCalculator = costCalculator;
        _metrics = metrics;
        _conversationStore = conversationStore;
        _toolCatalog = toolCatalog;
        _read = read;
        _provider = provider;
        _options = options;
        _cancellationTokenProvider = cancellationTokenProvider;
        _unitOfWorkManager = unitOfWorkManager;
        _clock = clock;
        _logger = logger;
    }

    public virtual async Task<AiAnswerDto> AskAsync(AiAskInput input)
    {
        Check.NotNull(input, nameof(input));
        var options = _options.Value;
        var assistant = options.Assistant;

        // 1. Who asks, and whether Ask AI itself is on for them.
        var caller = await _callerContextFactory.CreateAsync(input.Context?.TimeZone);
        await _accessGuard.EnsureAsync(caller, AiFeatureIds.AskId);

        // 2. The conversation.
        var conversationId = input.ConversationId ?? Guid.NewGuid();
        var consentStamp = AiConversationStore.ConsentStampOf(caller.Policy);
        AiConversationCacheItem conversation;
        if (input.ConversationId.HasValue)
        {
            var existing = await _conversationStore.FindAsync(caller.WorkspaceId, caller.UserId, conversationId);
            if (existing == null || existing.ConsentStamp != consentStamp)
            {
                throw new BusinessException(TrackerDomainErrorCodes.AiConversationExpired);
            }

            conversation = existing;
        }
        else
        {
            conversation = new AiConversationCacheItem { CreatedAtUtc = _clock.Now, ConsentStamp = consentStamp };
        }

        // 3. The question: redact secrets and emails, then replace visible member names with their handles.
        var sanitizedQuestion = UntrustedText.SanitizeQuestion(input.Message);
        var turn = new AiTurnContext(caller, conversation.MemberHandles, conversation.ProjectRefs);
        var roster = await _read.RosterAsync(caller);
        var resolution = AiNameResolver.Resolve(sanitizedQuestion, roster, turn.Members);

        if (resolution.AmbiguousNames.Count > 0)
        {
            return await RespondClarificationAsync(caller, conversation, conversationId, resolution.AmbiguousNames);
        }

        // Handles given by the resolver (or kept from earlier turns) are valid placeholder targets all turn, even
        // for a member no tool call happens to name again; this also records their real name for rendering, since
        // the resolver itself only knows the roster, not the turn.
        var rosterNames = roster.ToDictionary(r => r.UserId, r => r.DisplayName);
        foreach (var userId in turn.Members.Handles.Keys.ToList())
        {
            turn.AddMemberRef(userId, rosterNames.GetValueOrDefault(userId));
        }

        // 4. The page context becomes a server-built message; the burst limit.
        var contextMessage = AiAssistantPrompt.BuildContextMessage(caller, _clock.Now, input.Context);
        await _usageGate.EnsureBurstAsync(caller, AiFeatureIds.AskId);

        // 5. Quota, then the lock, slot and breaker.
        var estimatedTokens = (resolution.RewrittenQuestion.Length + contextMessage.Length) / 4 + assistant.MaxOutputTokens;
        await _usageGate.EnsureQuotaAsync(caller, AiFeatureIds.AskId, estimatedTokens);

        AiInsightRunner.EnsureNoTransaction(_unitOfWorkManager.Current);
        var callerToken = _cancellationTokenProvider.Token;
        await using var userLock = await _userLock.AcquireAsync(caller, AiFeatureIds.AskId, callerToken);
        using var slot = await _concurrencyGate.AcquireAsync(caller.WorkspaceId, AiFeatureIds.AskId, callerToken);

        // 6-9. The tools, the client, the call, validation and the fallback.
        var tools = _toolCatalog.For(caller, turn);
        var messages = BuildMessages(conversation, contextMessage, resolution.RewrittenQuestion);
        var client = BuildClient(turn, callerToken);
        var chatOptions = new ChatOptions
        {
            Tools = tools.ToList(),
            ToolMode = ChatToolMode.Auto,
            AllowMultipleToolCalls = true,
            MaxOutputTokens = assistant.MaxOutputTokens
        };

        using var deadline = System.Threading.CancellationTokenSource.CreateLinkedTokenSource(callerToken);
        deadline.CancelAfter(TimeSpan.FromSeconds(Math.Clamp(assistant.TurnTimeoutSeconds, 1, 300)));

        string? model = null;
        string answerText;
        var outcome = AiUsageOutcomes.Generated;
        string? errorCode = null;

        try
        {
            _circuitBreaker.ThrowIfOpen();
            var response = await GetResponseAsync(client, messages, chatOptions, deadline.Token);
            model = string.IsNullOrWhiteSpace(response.ModelId) ? null : response.ModelId;
            _circuitBreaker.RecordSuccess();

            var violations = AiGroundingValidator.ValidateText(
                "answer", response.Text, AiValidationProfile.Chat, turn.Facts.KnownIds);

            if (violations.Count > 0)
            {
                var repairMessages = AiPromptBuilder.BuildRepair(messages, response.Text, violations, jsonAnswer: false);
                var repaired = await GetResponseAsync(client, repairMessages, chatOptions, deadline.Token);
                var repairedViolations = AiGroundingValidator.ValidateText(
                    "answer", repaired.Text, AiValidationProfile.Chat, turn.Facts.KnownIds);
                if (repairedViolations.Count == 0)
                {
                    answerText = repaired.Text ?? string.Empty;
                }
                else
                {
                    answerText = FactsFallback(turn);
                    outcome = AiUsageOutcomes.Fallback;
                    errorCode = TrackerDomainErrorCodes.AiOutputInvalid;
                }
            }
            else
            {
                answerText = response.Text ?? string.Empty;
            }
        }
        catch (AiCircuitOpenException open)
        {
            throw AiErrors.ProviderUnavailable(AiErrors.CircuitOpenKind, open.RetryAfterSeconds, AiFeatureIds.AskId);
        }
        catch (OperationCanceledException) when (callerToken.IsCancellationRequested)
        {
            _circuitBreaker.RecordAbandoned();
            await RecordAsync(caller, turn, AiUsageOutcomes.Cancelled, model, null);
            throw;
        }
        catch (OperationCanceledException)
        {
            // The server's own deadline, not the caller: the call was sent, so it is a refusal worth a row.
            _circuitBreaker.RecordFailure(AiProviderErrorKind.Timeout);
            await RecordAsync(caller, turn, AiUsageOutcomes.Timeout, model, TrackerDomainErrorCodes.AiProviderUnavailable);
            throw AiErrors.ProviderUnavailable(AiProviderErrorMapper.ToUnavailableKind(AiProviderErrorKind.Timeout), null, AiFeatureIds.AskId);
        }
        catch (AiProviderException failure)
        {
            _circuitBreaker.RecordFailure(failure.Kind);
            var failureOutcome = failure.Kind == AiProviderErrorKind.Timeout ? AiUsageOutcomes.Timeout : AiUsageOutcomes.ProviderError;
            await RecordAsync(caller, turn, failureOutcome, model, TrackerDomainErrorCodes.AiProviderUnavailable);
            throw AiErrors.ProviderUnavailable(AiProviderErrorMapper.ToUnavailableKind(failure.Kind), failure.RetryAfterSeconds, AiFeatureIds.AskId);
        }

        // 10. Render, build sources, store the turn.
        var bundle = BundleOf(turn);
        var renderedText = AiPlaceholderRenderer.Render(answerText, bundle);
        var referencedFactIds = AiGroundingValidator.PlaceholderIds(answerText).Where(id => turn.Facts.Facts.ContainsKey(id)).ToList();
        var sources = BuildSources(turn, referencedFactIds);
        var facts = AiFeatureOutput.RenderFacts(referencedFactIds, bundle);

        conversation.MemberHandles = turn.Members.Handles
            .Where(h => h.Key != caller.UserId)
            .ToDictionary(h => h.Key, h => h.Value);
        conversation.ProjectRefs = new Dictionary<Guid, string>(turn.ProjectRefs);
        conversation.Turns.Add(new AiConversationTurn
        {
            QuestionText = resolution.RewrittenQuestion,
            AnswerText = RenderFactsOnly(answerText, turn)
        });
        if (conversation.Turns.Count > assistant.MaxHistoryTurns)
        {
            conversation.Turns.RemoveRange(0, conversation.Turns.Count - assistant.MaxHistoryTurns);
        }

        await _conversationStore.SaveAsync(caller.WorkspaceId, caller.UserId, conversationId, conversation, assistant.ConversationTtlMinutes);
        var resultId = await RecordAsync(caller, turn, outcome, model, errorCode);

        return new AiAnswerDto
        {
            Meta = BuildMeta(resultId, outcome, model),
            ConversationId = conversationId,
            Answer = renderedText,
            Sources = sources,
            Facts = facts,
            IsClarification = false
        };
    }

    private async Task<AiAnswerDto> RespondClarificationAsync(
        AiCallerContext caller, AiConversationCacheItem conversation, Guid conversationId, IReadOnlyList<string> ambiguousNames)
    {
        var who = string.Join(" or ", ambiguousNames.Select(n => $"\"{n}\""));
        var answer = $"More than one person matches {who}. Could you say their full name, or pick them from the team list?";
        await _conversationStore.SaveAsync(caller.WorkspaceId, caller.UserId, conversationId, conversation, _options.Value.Assistant.ConversationTtlMinutes);

        // No model call, no tokens: the ledger row records that, but still counts the turn.
        await _recorder.RecordAsync(new AiUsageEntry(
            caller.WorkspaceId, caller.UserId, AiFeatureIds.AskId, AiAssistantPrompt.Version, AiUsageRecord.SelfAudience,
            ProviderName(), null, AiUsageOutcomes.Clarification));

        return new AiAnswerDto
        {
            Meta = new AiResultMetaDto
            {
                FeatureId = AiFeatureIds.AskId,
                Status = AiResultStatuses.Generated,
                GeneratedAt = _clock.Now,
                PromptVersion = AiAssistantPrompt.Version,
                Disclaimer = string.Empty
            },
            ConversationId = conversationId,
            Answer = answer,
            IsClarification = true
        };
    }

    private IList<ChatMessage> BuildMessages(AiConversationCacheItem conversation, string contextMessage, string question)
    {
        var messages = new List<ChatMessage>
        {
            new(ChatRole.System, AiAssistantPrompt.V1),
            new(ChatRole.System, contextMessage)
        };

        foreach (var turn in conversation.Turns)
        {
            messages.Add(new ChatMessage(ChatRole.User, turn.QuestionText));
            messages.Add(new ChatMessage(ChatRole.Assistant, turn.AnswerText));
        }

        messages.Add(new ChatMessage(ChatRole.User, question));
        return messages;
    }

    private IChatClient BuildClient(AiTurnContext turn, System.Threading.CancellationToken callerToken)
    {
        var baseClient = _provider.GetClient(AiFeatureIds.AskId)
                          ?? throw new BusinessException(TrackerDomainErrorCodes.AiNotConfigured).WithData("featureId", AiFeatureIds.AskId);
        var assistant = _options.Value.Assistant;

        return new ChatClientBuilder(baseClient)
            .UseFunctionInvocation(NullLoggerFactory.Instance, f =>
            {
                f.MaximumIterationsPerRequest = Math.Max(1, assistant.MaxToolRounds);
                f.MaximumConsecutiveErrorsPerRequest = 2;
                f.AllowConcurrentInvocation = false;
                f.IncludeDetailedErrors = false;
                f.TerminateOnUnknownCalls = true;
            })
            .Use(inner => new UsageMeteringChatClient(inner, turn.Usage))
            .Build();
    }

    private async Task<ChatResponse> GetResponseAsync(
        IChatClient client, IEnumerable<ChatMessage> messages, ChatOptions options, System.Threading.CancellationToken token)
    {
        try
        {
            return await client.GetResponseAsync(messages, options, token);
        }
        catch (Exception e) when (e is not AiProviderException and not OperationCanceledException)
        {
            _logger.LogWarning("AI assist's ask call failed with {ExceptionType}.", e.GetType().Name);
            throw new AiProviderException(AiProviderErrorKind.ServerError);
        }
    }

    private static string FactsFallback(AiTurnContext turn)
    {
        var facts = turn.Facts.Facts.Take(8)
            .Select(f => $"- {f.Value.Label}: {f.Value.Display}");
        return "I couldn't put together a reliable answer. These are the figures I looked up:\n" + string.Join("\n", facts);
    }

    /// <summary>Only facts are substituted when a turn's answer is folded into history; member, project, app and
    /// task handles stay, since they still resolve in a later turn of the same conversation (their names and quoted
    /// text are never written to storage).</summary>
    private static string RenderFactsOnly(string? answerText, AiTurnContext turn)
    {
        if (string.IsNullOrEmpty(answerText))
        {
            return string.Empty;
        }

        return AiGroundingValidator.Placeholder.Replace(
            answerText, m => turn.Facts.Facts.TryGetValue(m.Groups[1].Value, out var fact) ? fact.Display : m.Value);
    }

    private static List<AiSourceDto> BuildSources(AiTurnContext turn, IReadOnlyList<string> referencedFactIds)
    {
        var usedResultIds = new HashSet<string>(
            referencedFactIds.Select(turn.Facts.ResultOf).Where(id => id != null).Select(id => id!), StringComparer.Ordinal);

        return turn.Citations
            .Where(c => usedResultIds.Contains(c.ResultId))
            .Select(c => new AiSourceDto { Label = c.Label, RouteId = c.RouteId, RouteArg = c.RouteArg })
            .Take(6)
            .ToList();
    }

    /// <summary>The bundle the final, user-facing answer is rendered from: this turn's facts exactly as registered
    /// (same ids the model was given), plus the real names and quoted text behind every handle used this turn. Built
    /// directly (not through <see cref="AiBundleWriter"/>), so fact ids are never renumbered.</summary>
    private static AiContextBundle BundleOf(AiTurnContext turn)
    {
        var resolution = new Dictionary<string, AiAlias>(StringComparer.Ordinal);
        foreach (var (userId, name) in turn.MemberNames)
        {
            if (turn.Members.TryGetHandle(userId, out var handle))
            {
                resolution[handle] = new AiAlias(userId, name);
            }
        }

        return new AiContextBundle(
            turn.Caller.UserId,
            turn.Facts.Facts.ToList(),
            new System.Text.Json.Nodes.JsonObject(),
            turn.Labels.ToList(),
            Array.Empty<string>(),
            Array.Empty<string>(),
            new System.Text.Json.Nodes.JsonArray(),
            resolution);
    }

    private AiResultMetaDto BuildMeta(Guid resultId, string outcome, string? model) => new()
    {
        ResultId = resultId,
        FeatureId = AiFeatureIds.AskId,
        Status = outcome == AiUsageOutcomes.Fallback ? AiResultStatuses.Fallback : AiResultStatuses.Generated,
        GeneratedAt = _clock.Now,
        Model = model,
        PromptVersion = AiAssistantPrompt.Version,
        Disclaimer = string.Empty
    };

    private async Task<Guid> RecordAsync(AiCallerContext caller, AiTurnContext turn, string outcome, string? model, string? errorCode)
    {
        var usage = turn.Usage.Snapshot;
        var cost = usage.IsEmpty ? null : _costCalculator.Estimate(model, usage.InputTokens, usage.CachedInputTokens, usage.OutputTokens);
        var toolCalls = string.Join(",", turn.Citations.GroupBy(c => c.Tool).Select(g => $"{g.Key}:{g.Count()}"));

        var id = await _recorder.RecordAsync(new AiUsageEntry(
            caller.WorkspaceId, caller.UserId, AiFeatureIds.AskId, AiAssistantPrompt.Version, AiUsageRecord.SelfAudience,
            ProviderName(), model, outcome)
        {
            DataCategories = new[] { AiDataCategories.Durations, AiDataCategories.Times, AiDataCategories.HelpContent },
            ErrorCode = errorCode,
            Usage = usage,
            EstimatedCostUsd = cost,
            ToolCalls = string.IsNullOrEmpty(toolCalls) ? null : toolCalls
        });

        _metrics.Request(AiFeatureIds.AskId, outcome);
        _metrics.Tokens(usage.InputTokens, usage.OutputTokens);
        return id;
    }

    private string ProviderName()
    {
        var name = _provider.Descriptor?.Provider;
        return string.IsNullOrWhiteSpace(name) ? _options.Value.Provider : name;
    }
}
