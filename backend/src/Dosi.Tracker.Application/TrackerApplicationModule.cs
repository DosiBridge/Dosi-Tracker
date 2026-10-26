using System;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Provider;
using Volo.Abp.PermissionManagement;
using Volo.Abp.SettingManagement;
using Volo.Abp.Account;
using Volo.Abp.Identity;
using Volo.Abp.Mapperly;
using Volo.Abp.FeatureManagement;
using Volo.Abp.Modularity;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerDomainModule),
    typeof(TrackerApplicationContractsModule),
    typeof(AbpPermissionManagementApplicationModule),
    typeof(AbpFeatureManagementApplicationModule),
    typeof(AbpIdentityApplicationModule),
    typeof(AbpAccountApplicationModule),
    typeof(AbpSettingManagementApplicationModule)
    )]
public class TrackerApplicationModule : AbpModule
{
    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        // AI assist is off unless a host supplies a provider (the API host's OpenAI provider, a test's fake).
        context.Services.TryAddSingleton<IAiChatClientProvider, NullAiChatClientProvider>();

        // Per API instance: the provider breaker, the concurrency slots and the metrics are shared by every request.
        context.Services.TryAddSingleton(sp =>
            new AiCircuitBreaker(sp.GetRequiredService<ILogger<AiCircuitBreaker>>(), TimeProvider.System));
        context.Services.TryAddSingleton<AiConcurrencyGate>();
        context.Services.TryAddSingleton<AiMetrics>();
    }
}
