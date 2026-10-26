using System;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Context;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Validation;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>Why a tool returned nothing useful (spec 10.2); these never throw back to the model, they are the tool's
/// own JSON result.</summary>
public static class AiToolErrors
{
    public const string UnknownMember = "unknown_member";
    public const string UnknownProject = "unknown_project";
    public const string NotPermitted = "not_permitted";
    public const string NotFound = "not_found";
    public const string InvalidRange = "invalid_range";
    public const string Unavailable = "unavailable";
    public const string BudgetExhausted = "budget_exhausted";

    public static JsonObject Of(string code) => new() { ["error"] = code };
}

/// <summary>
/// What every Ask AI tool call runs through (spec 10.2): the call and character budgets, the 5-second per-call limit,
/// mapping an app-service exception to one of <see cref="AiToolErrors"/> (never the exception itself, never its
/// message), and registering the result's citation once it is kept. A tool whose handler already decided the answer
/// is an error (an unknown handle, a permission the caller lacks to see another member) returns that error itself,
/// before this gate is reached; this only guards what the handler could not have decided on its own.
/// </summary>
public class AiToolGate : ITransientDependency
{
    private static readonly TimeSpan PerCallTimeout = TimeSpan.FromSeconds(5);

    private readonly IOptions<AiOptions> _options;
    private readonly ILogger<AiToolGate> _logger;

    public AiToolGate(IOptions<AiOptions> options, ILogger<AiToolGate> logger)
    {
        _options = options;
        _logger = logger;
    }

    /// <param name="turn">The turn the call belongs to.</param>
    /// <param name="toolName">The tool's name (for the log line only; never the arguments or the result).</param>
    /// <param name="label">The citation's label, used only if the result is kept.</param>
    /// <param name="routeId">An allowlisted route id the citation may offer, or null.</param>
    /// <param name="routeArg">The route's argument (a project id), when it needs one.</param>
    /// <param name="handler">Builds the result, given the result id ("c1" …) to tag its facts with. May return an
    /// error object of its own (<see cref="AiToolErrors"/>) instead of throwing.</param>
    public virtual async Task<JsonObject> RunAsync(
        AiTurnContext turn, string toolName, string label, string? routeId, string? routeArg, Func<string, Task<JsonObject>> handler)
    {
        var limits = _options.Value.Assistant;
        if (!turn.TryReserveCall(limits.MaxToolCallsPerTurn))
        {
            return AiToolErrors.Of(AiToolErrors.BudgetExhausted);
        }

        var resultId = turn.NextResultId();
        JsonObject result;
        try
        {
            var work = handler(resultId);
            var delay = Task.Delay(PerCallTimeout);
            var winner = await Task.WhenAny(work, delay);
            if (winner == delay)
            {
                return AiToolErrors.Of(AiToolErrors.Unavailable);
            }

            result = await work;
        }
        catch (AbpAuthorizationException)
        {
            return AiToolErrors.Of(AiToolErrors.NotPermitted);
        }
        catch (EntityNotFoundException)
        {
            return AiToolErrors.Of(AiToolErrors.NotFound);
        }
        catch (BusinessException be) when (
            be.Code == TrackerDomainErrorCodes.ReportRangeInvalid || be.Code == TrackerDomainErrorCodes.AiRangeTooLarge)
        {
            return AiToolErrors.Of(AiToolErrors.InvalidRange);
        }
        catch (AbpValidationException)
        {
            return AiToolErrors.Of(AiToolErrors.InvalidRange);
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (Exception e)
        {
            _logger.LogWarning("Ask AI tool {Tool} failed with {ExceptionType}.", toolName, e.GetType().Name);
            return AiToolErrors.Of(AiToolErrors.Unavailable);
        }

        if (result.ContainsKey("error"))
        {
            return result;
        }

        result["result"] = resultId;
        var text = result.ToJsonString();
        if (text.Length > limits.MaxToolResultChars || !turn.TryReserveChars(text.Length, limits.MaxTurnToolChars))
        {
            return AiToolErrors.Of(AiToolErrors.BudgetExhausted);
        }

        turn.AddCitation(new AiCitation(resultId, toolName, label, routeId, routeArg));
        return result;
    }

    /// <summary>
    /// The member a "member" argument names: "ME" or absent means the caller decides nothing (null; the read layer's
    /// own scoping then answers for the team when the caller may see it, or for the caller otherwise — see
    /// <see cref="Read.AiReadLayer.ScopedUserId"/>), except that "ME" always means the caller explicitly, even when
    /// they could see the team. A handle resolves to that member; an unknown one is an error. Asking about a member
    /// the caller may not see is left to the read layer, which refuses with <see cref="AbpAuthorizationException"/>
    /// (mapped to <see cref="AiToolErrors.NotPermitted"/> by <see cref="RunAsync"/>), never silently narrowed.
    /// </summary>
    public static (Guid? UserId, JsonObject? Error) ResolveMember(AiTurnContext turn, string? memberArg)
    {
        if (string.IsNullOrWhiteSpace(memberArg))
        {
            return (null, null);
        }

        if (memberArg == AiPseudonymizer.SelfHandle)
        {
            return (turn.Caller.UserId, null);
        }

        var userId = turn.ResolveMemberRef(memberArg);
        return userId == null ? (null, AiToolErrors.Of(AiToolErrors.UnknownMember)) : (userId, null);
    }

    /// <summary>"self" when the effective query is only the caller's own data (a caller who cannot see the team, or
    /// one who asked about themselves by name); "team" otherwise (the team as a whole, or another named member).</summary>
    public static string ScopeOf(AiTurnContext turn, Guid? resolvedUserId) =>
        !turn.Caller.CanSeeTeam || resolvedUserId == turn.Caller.UserId ? "self" : "team";

    /// <summary>The project a "project" argument names, or an <see cref="AiToolErrors.UnknownProject"/> error when the
    /// handle was never given this turn.</summary>
    public static (Guid? ProjectId, JsonObject? Error) ResolveProject(AiTurnContext turn, string? projectArg)
    {
        if (string.IsNullOrWhiteSpace(projectArg))
        {
            return (null, null);
        }

        var projectId = turn.ResolveProjectRef(projectArg);
        return projectId == null ? (null, AiToolErrors.Of(AiToolErrors.UnknownProject)) : (projectId, null);
    }
}
