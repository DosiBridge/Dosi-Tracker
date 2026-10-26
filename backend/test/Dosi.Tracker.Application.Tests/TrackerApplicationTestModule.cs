using Dosi.Tracker.Emailing;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Volo.Abp.Emailing;
using Volo.Abp.Identity.AspNetCore;
using Volo.Abp.Modularity;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerApplicationModule),
    typeof(TrackerDomainTestModule),
    // ABP's ASP.NET Identity token providers (password reset, email confirmation), as the API host has them through
    // ABP's account web module.
    typeof(AbpIdentityAspNetCoreModule)
)]
public class TrackerApplicationTestModule : AbpModule
{
    public override void PreConfigureServices(ServiceConfigurationContext context)
    {
        // The tests authenticate with their own scheme; Identity's sign-in cookies are not wanted.
        PreConfigure<AbpIdentityAspNetCoreOptions>(options => options.ConfigureAuthentication = false);
    }

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        // The token providers' keys: an ephemeral key ring per test application.
        context.Services.AddDataProtection();

        // No test sends real email: every email is recorded, and email delivery counts as configured.
        var emails = new CapturingEmailSender();
        context.Services.Replace(ServiceDescriptor.Singleton(emails));
        context.Services.Replace(ServiceDescriptor.Singleton<IEmailSender>(emails));
        context.Services.Replace(ServiceDescriptor.Singleton<IEmailDeliveryChecker>(emails));
    }
}
