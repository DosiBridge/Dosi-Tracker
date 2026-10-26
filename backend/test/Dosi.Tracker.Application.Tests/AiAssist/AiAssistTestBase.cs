using System;
using System.Collections.Generic;
using System.Security.Claims;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Fakes;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.Security.Claims;
using Volo.Abp.Timing;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// Shared set-up of the AI assist tests: a configured fake provider (whose client throws on any call, so a test that
/// reaches a model fails) and helpers for members, the workspace's AI settings and ledger rows. The test principal
/// (<see cref="CallerId"/>) works in the test workspace; <see cref="AsUserAsync{T}"/> acts as someone else.
/// </summary>
public abstract class AiAssistTestBase<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    protected static readonly Guid CallerId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    protected static readonly Guid WorkspaceId = TrackerTestConsts.WorkspaceId;

    /// <summary>The provider every AI path gets (unless <see cref="UseFakeProvider"/> is false).</summary>
    protected FakeAiChatClientProvider Provider { get; } = new();

    protected virtual bool UseFakeProvider => true;

    protected AiOptions AiOptions => GetRequiredService<IOptions<AiOptions>>().Value;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
        if (UseFakeProvider)
        {
            services.Replace(ServiceDescriptor.Singleton<IAiChatClientProvider>(Provider));
        }
    }

    /// <summary>An account that is an active member of the test workspace with <paramref name="role"/>.</summary>
    protected async Task<Guid> CreateMemberAsync(string userName, string role, Guid? id = null, string? name = null)
    {
        var userId = id ?? Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(userId, userName, $"{userName}@acme.test") { Name = name ?? userName };
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
        });
        await ServiceProvider.AddWorkspaceMemberAsync(userId, role);
        return userId;
    }

    /// <summary>Makes the test principal an active member of the test workspace with <paramref name="role"/>.</summary>
    protected Task<Guid> SignInAsAsync(string role, string name = "Caller Person") =>
        CreateMemberAsync("ai.caller", role, CallerId, name);

    /// <summary>Gives <paramref name="userId"/> a membership of a new project with this project role.</summary>
    protected async Task<Guid> AddProjectMembershipAsync(Guid userId, string projectRole)
    {
        var projectId = Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>()
                .InsertAsync(new Project(projectId, WorkspaceId, "Project " + projectId.ToString("N")[..6]), autoSave: true);
            await GetRequiredService<IRepository<ProjectMember, Guid>>()
                .InsertAsync(new ProjectMember(Guid.NewGuid(), WorkspaceId, projectId, userId, projectRole), autoSave: true);
        });
        return projectId;
    }

    protected Task<AiSettingsChange> SetPolicyAsync(
        bool enabled = true, bool managers = true, bool selfService = true, bool titles = false, bool notes = false,
        Guid? workspaceId = null) =>
        WithUnitOfWorkAsync(() => GetRequiredService<AiSettingsManager>().UpdateAsync(
            workspaceId ?? WorkspaceId, CallerId, new AiWorkspaceSettingsUpdate(enabled, managers, selfService, titles, notes),
            GetRequiredService<AiSettingsManager>().CurrentDisclosureVersion));

    protected Task SetPlatformEnabledAsync(bool enabled) =>
        WithUnitOfWorkAsync(() => GetRequiredService<AiSettingsManager>().SetPlatformEnabledAsync(enabled));

    /// <summary>Gives the test workspace a subscription with this status on a new plan.</summary>
    protected Task SetSubscriptionAsync(string status, string planName = "Ai Test Plan", Guid? workspaceId = null)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var plan = new Plan(Guid.NewGuid(), planName, 5m, 0, 0);
            await GetRequiredService<IRepository<Plan, Guid>>().InsertAsync(plan, autoSave: true);
            var id = workspaceId ?? WorkspaceId;
            using (GetRequiredService<ICurrentWorkspace>().Change(id))
            {
                await GetRequiredService<IRepository<Subscription, Guid>>().InsertAsync(
                    new Subscription(Guid.NewGuid(), id, plan.Id) { Status = status }, autoSave: true);
            }
        });
    }

    /// <summary>Writes a ledger row (as a feature's request would).</summary>
    protected Task<AiUsageRecord> InsertUsageAsync(
        Guid userId,
        string featureId = AiFeatureIds.PeriodSummaryId,
        string outcome = AiUsageOutcomes.Generated,
        bool cacheHit = false,
        int? inputTokens = 100,
        int? outputTokens = 20,
        decimal? cost = null,
        DateTime? at = null,
        Guid? workspaceId = null)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var record = new AiUsageRecord(Guid.NewGuid(), workspaceId ?? WorkspaceId, userId, featureId, featureId + ".v1",
                AiUsageRecord.SelfAudience, AiOptions.ProviderOpenAi, "gpt-5.4-mini", outcome, cacheHit: cacheHit);
            record.SetUsage(inputTokens, outputTokens, estimatedCostUsd: cost);
            if (at.HasValue)
            {
                ObjectHelper.TrySetProperty(record, r => r.CreationTime, () => at.Value);
            }

            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await GetRequiredService<IRepository<AiUsageRecord, Guid>>().InsertAsync(record, autoSave: true);
            }
        });
    }

    protected DateTime Now => GetRequiredService<IClock>().Now;

    /// <summary>Runs <paramref name="action"/> as the signed-in <paramref name="userId"/>, in the test workspace.</summary>
    protected async Task<T> AsUserAsync<T>(Guid userId, Func<Task<T>> action)
    {
        var claims = new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) };
        using (GetRequiredService<ICurrentPrincipalAccessor>().Change(new ClaimsPrincipal(new ClaimsIdentity(claims, "TestAuth"))))
        {
            return await action();
        }
    }

    protected async Task AsUserAsync(Guid userId, Func<Task> action)
    {
        await AsUserAsync(userId, async () =>
        {
            await action();
            return true;
        });
    }
}
