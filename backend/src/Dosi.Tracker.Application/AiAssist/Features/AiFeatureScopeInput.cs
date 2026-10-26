using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using Dosi.Tracker.AiAssist.Read;
using Volo.Abp.Validation;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>Input rules the insight cards share.</summary>
public static class AiFeatureScopeInput
{
    /// <summary>True for "team", false for "self"; anything else is refused, never read as one of them.</summary>
    /// <exception cref="AbpValidationException">Another value.</exception>
    public static bool IsTeam(string? scope)
    {
        return scope switch
        {
            AiFeatureScopes.Team => true,
            AiFeatureScopes.Self => false,
            _ => throw new AbpValidationException(
                "The scope is not valid.",
                new List<ValidationResult> { new("Use \"team\" or \"self\".", new[] { "scope" }) })
        };
    }

    /// <summary>The member's display name in the summary's rows (empty when they have none).</summary>
    public static string NameOf(AiSummary summary, Guid userId) =>
        summary.Members.FirstOrDefault(m => m.UserId == userId)?.DisplayName ?? string.Empty;
}
