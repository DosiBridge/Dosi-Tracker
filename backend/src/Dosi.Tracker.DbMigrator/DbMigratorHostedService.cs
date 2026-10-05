using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Dosi.Tracker.Data;
using Serilog;
using Volo.Abp;
using Volo.Abp.Data;

namespace Dosi.Tracker.DbMigrator;

public class DbMigratorHostedService : IHostedService
{
    private readonly IHostApplicationLifetime _hostApplicationLifetime;
    private readonly IConfiguration _configuration;
    private readonly IHostEnvironment _hostEnvironment;

    public DbMigratorHostedService(
        IHostApplicationLifetime hostApplicationLifetime,
        IConfiguration configuration,
        IHostEnvironment hostEnvironment)
    {
        _hostApplicationLifetime = hostApplicationLifetime;
        _configuration = configuration;
        _hostEnvironment = hostEnvironment;
    }

    public async Task StartAsync(CancellationToken cancellationToken)
    {
        using (var application = await AbpApplicationFactory.CreateAsync<TrackerDbMigratorModule>(options =>
        {
           options.Services.ReplaceConfiguration(_configuration);
           // The host's environment (DOTNET_ENVIRONMENT), which ABP would otherwise always take to be Production: the
           // first host administrator may get the template password in Development only.
           options.Environment = _hostEnvironment.EnvironmentName;
           options.UseAutofac();
           options.Services.AddLogging(c => c.AddSerilog());
           options.AddDataMigrationEnvironment();
        }))
        {
            await application.InitializeAsync();

            await application
                .ServiceProvider
                .GetRequiredService<TrackerDbMigrationService>()
                .MigrateAsync();

            await application.ShutdownAsync();

            _hostApplicationLifetime.StopApplication();
        }
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        return Task.CompletedTask;
    }
}
