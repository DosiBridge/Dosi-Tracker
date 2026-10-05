using Microsoft.Extensions.DependencyInjection;
using Volo.Abp.SettingManagement;

namespace Dosi.Tracker.Settings;

/// <summary>
/// ABP saves the static setting definitions to the database on a background task whenever an application starts.
/// Each integration test starts an application on one in-memory SQLite connection, so that task can still be using
/// the connection while the test queries it or closes it at shutdown (seen as a NullReferenceException in
/// SqliteConnection.Close). Tests only read and write setting values, never the stored definitions, so it is off: the
/// EF Core test module turns it off for every test application (as it does for features and permissions), and the
/// test classes that work with settings state it here as well.
/// </summary>
public static class StaticSettingSavingTestExtensions
{
    public static IServiceCollection DisableStaticSettingSaving(this IServiceCollection services)
    {
        return services.Configure<SettingManagementOptions>(options =>
        {
            options.SaveStaticSettingsToDatabase = false;
            options.IsDynamicSettingStoreEnabled = false;
        });
    }
}
