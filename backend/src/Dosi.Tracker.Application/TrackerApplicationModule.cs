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
    }
}
