using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Validation;

namespace Dosi.Tracker.AiAssist.Read;

/// <summary>
/// What an AI request may be about, checked before anything is read. It refuses and never narrows: a member asking
/// about the team or someone else gets a 403, never a quieter answer about themselves, so nobody can mistake their own
/// data for the team's.
/// </summary>
public class AiScopeGuard : ITransientDependency
{
    public const string TeamScopeMessage =
        "Team AI summaries require the Tracker.Reporting and Tracker.Activities.ViewAll permissions.";

    /// <summary>A DST change inside a range of whole local days may lengthen it by up to this much.</summary>
    public static readonly TimeSpan RangeSlack = TimeSpan.FromHours(2);

    /// <summary>The reports report-explanation explains (the web app's catalog without payroll).</summary>
    public static readonly IReadOnlyList<string> ReportSlugs = new[]
    {
        "time-activity", "weekly", "projects", "productivity", "apps-urls", "attendance", "devices", "work-report", "analytics"
    };

    private readonly WorkspaceMembershipManager _memberships;

    public AiScopeGuard(WorkspaceMembershipManager memberships)
    {
        _memberships = memberships;
    }

    /// <summary>The team, or another member, needs an owner or manager holding ViewAll and Reporting.</summary>
    /// <exception cref="AbpAuthorizationException">The caller only sees their own data.</exception>
    public virtual void EnsureTeamScope(AiCallerContext caller)
    {
        Check.NotNull(caller, nameof(caller));
        if (!caller.CanSeeTeam)
        {
            throw new AbpAuthorizationException(TeamScopeMessage);
        }
    }

    /// <summary>
    /// The member a request is about: the caller, or (for a caller who sees the team) another active member of the
    /// workspace.
    /// </summary>
    /// <returns>The member's id (the caller's when <paramref name="userId"/> is null).</returns>
    /// <exception cref="AbpAuthorizationException">Another member, asked about by a caller who only sees themselves.</exception>
    /// <exception cref="BusinessException"><c>Tracker:InvalidUser</c>: not an active member of the workspace.</exception>
    public virtual async Task<Guid> EnsureMemberTargetAsync(AiCallerContext caller, Guid? userId)
    {
        Check.NotNull(caller, nameof(caller));
        if (!userId.HasValue || userId.Value == caller.UserId)
        {
            return caller.UserId;
        }

        EnsureTeamScope(caller);

        if (!await _memberships.IsActiveMemberAsync(caller.WorkspaceId, userId.Value))
        {
            throw new BusinessException(TrackerDomainErrorCodes.InvalidUser).WithData("userId", userId.Value);
        }

        return userId.Value;
    }

    /// <summary>A UTC range [from, to) of at most <paramref name="maxDays"/> days (a DST change inside it does not count).</summary>
    /// <exception cref="BusinessException"><c>Tracker:ReportRangeInvalid</c> when it is empty or reversed;
    /// <c>Tracker:AiRangeTooLarge</c> (data maxDays) when it is too long.</exception>
    public virtual void EnsureRange(DateTime from, DateTime to, int maxDays)
    {
        if (to <= from)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ReportRangeInvalid);
        }

        if (to - from > TimeSpan.FromDays(maxDays) + RangeSlack)
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiRangeTooLarge).WithData("maxDays", maxDays);
        }
    }

    /// <summary>A report report-explanation may explain (<see cref="ReportSlugs"/>, exact).</summary>
    /// <returns>The slug, trimmed.</returns>
    /// <exception cref="AbpValidationException">Any other slug, payroll included.</exception>
    public virtual string EnsureReportSlug(string? slug)
    {
        var value = slug?.Trim();
        foreach (var known in ReportSlugs)
        {
            if (string.Equals(known, value, StringComparison.Ordinal))
            {
                return known;
            }
        }

        throw new AbpValidationException(
            "This report cannot be explained by AI assist.",
            new List<ValidationResult> { new("Choose one of the supported reports.", new[] { "reportSlug" }) });
    }
}
