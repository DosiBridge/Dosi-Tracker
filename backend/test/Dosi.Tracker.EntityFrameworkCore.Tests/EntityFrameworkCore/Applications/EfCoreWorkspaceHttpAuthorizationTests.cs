using System.Net;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Mvc;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Volo.Abp.AspNetCore.Mvc;
using Volo.Abp.Modularity;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

[DependsOn(
    typeof(TrackerEntityFrameworkCoreTestModule),
    typeof(TrackerHttpApiModule),
    typeof(AbpAspNetCoreMvcModule),
    typeof(Volo.Abp.Autofac.AbpAutofacModule)
)]
public class TrackerHttpApiServerTestModule : AbpModule
{
    public override void PreConfigureServices(ServiceConfigurationContext context)
    {
        PreConfigure<IMvcBuilder>(mvcBuilder =>
        {
            mvcBuilder.AddApplicationPartIfNotExists(typeof(TrackerHttpApiModule).Assembly);
        });
    }

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        Configure<AbpAspNetCoreMvcOptions>(options =>
        {
            options.ConventionalControllers.Create(typeof(TrackerApplicationModule).Assembly);
        });

        Configure<MvcOptions>(options =>
        {
            options.Filters.Add(new WorkspaceRequiredFilter(
                typeof(TrackerApplicationModule).Assembly,
                typeof(TrackerHttpApiModule).Assembly));
        });

        context.Services.AddAlwaysDisableUnitOfWorkTransaction();
    }
}

public class EfCoreWorkspaceHttpAuthorizationTests : WorkspaceHttpAuthorizationTests<TrackerHttpApiServerTestModule>
{
    protected override void ConfigureTestServerServices(IServiceCollection services)
    {
        var db = ServiceProvider.GetRequiredService<TrackerDbContext>();
        var conn = db.Database.GetDbConnection();
        services.Configure<Volo.Abp.EntityFrameworkCore.AbpDbContextOptions>(options =>
        {
            options.Configure(ctx =>
            {
                ctx.DbContextOptions.UseSqlite(conn);
            });
        });
        services.AddAlwaysDisableUnitOfWorkTransaction();
    }
}
