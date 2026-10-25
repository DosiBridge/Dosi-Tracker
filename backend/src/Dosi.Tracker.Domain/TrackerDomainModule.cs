using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Dosi.Tracker.Localization;
using Dosi.Tracker.MultiTenancy;
using System;
using Volo.Abp.Localization;
using Volo.Abp.Timing;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.PermissionManagement.Identity;
using Volo.Abp.SettingManagement;
using Volo.Abp.BlobStoring.Database;
using Volo.Abp.Caching;
using Volo.Abp.OpenIddict;
using Volo.Abp.PermissionManagement.OpenIddict;
using Volo.Abp.AuditLogging;
using Volo.Abp.BackgroundJobs;
using Volo.Abp.BackgroundWorkers;
using Volo.Abp.Emailing;
using Volo.Abp.FeatureManagement;
using Volo.Abp.Identity;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerDomainSharedModule),
    typeof(AbpAuditLoggingDomainModule),
    typeof(AbpCachingModule),
    typeof(AbpBackgroundJobsDomainModule),
    typeof(AbpFeatureManagementDomainModule),
    typeof(AbpPermissionManagementDomainIdentityModule),
    typeof(AbpPermissionManagementDomainOpenIddictModule),
    typeof(AbpSettingManagementDomainModule),
    typeof(AbpEmailingModule),
    typeof(AbpIdentityDomainModule),
    typeof(AbpOpenIddictDomainModule),
    typeof(AbpBackgroundWorkersModule),
    typeof(BlobStoringDatabaseDomainModule)
    )]
public class TrackerDomainModule : AbpModule
{
    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        Configure<AbpMultiTenancyOptions>(options =>
        {
            options.IsEnabled = MultiTenancyConsts.IsEnabled;
        });

        // One account per person: the email identifies them across every workspace they belong to.
        Configure<Microsoft.AspNetCore.Identity.IdentityOptions>(options =>
        {
            options.User.RequireUniqueEmail = true;
        });

        Configure<Volo.Abp.Settings.AbpSettingOptions>(options =>
        {
            InsertBefore(options.ValueProviders, typeof(Volo.Abp.Settings.UserSettingValueProvider), typeof(Settings.WorkspaceSettingValueProvider));
        });

        Configure<Volo.Abp.Authorization.Permissions.AbpPermissionOptions>(options =>
        {
            options.ValueProviders.Add<Workspaces.WorkspaceRolePermissionValueProvider>();
        });

        Configure<SettingManagementOptions>(options =>
        {
            InsertBefore(options.Providers, typeof(UserSettingManagementProvider), typeof(Settings.WorkspaceSettingManagementProvider));
        });

        // Agents submit UTC and reports bucket by UTC day, so the server clock must be UTC too:
        // otherwise CreationTime / TrialEndsAt are stored in the host's local time and serialized
        // without an offset, and browsers misread them by the server's UTC offset.
        Configure<AbpClockOptions>(options =>
        {
            options.Kind = DateTimeKind.Utc;
        });

        // Bound from the application's configuration when the options are first read, so a host that replaces its
        // configuration after the modules are configured still gets its own values.
        context.Services.AddOptions<Billing.BillingOptions>()
            .Configure<IConfiguration>((options, configuration) =>
                configuration.GetSection(Billing.BillingOptions.SectionName).Bind(options));
        context.Services.AddOptions<Erasure.DataErasureOptions>()
            .Configure<IConfiguration>((options, configuration) =>
                configuration.GetSection(Erasure.DataErasureOptions.SectionName).Bind(options));

        // Email goes through Emailing.TrackerEmailSender, which sends nothing (and only logs that it did not) until an SMTP
        // host is configured, so a deployment without SMTP — and every development machine — never fails on email.
    }

    private static void InsertBefore<T>(Volo.Abp.Collections.ITypeList<T> list, Type before, Type type)
    {
        var index = list.IndexOf(before);
        if (index < 0)
        {
            list.Add(type);
        }
        else
        {
            list.Insert(index, type);
        }
    }
}
