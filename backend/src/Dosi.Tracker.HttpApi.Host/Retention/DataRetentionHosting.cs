using System;
using System.Collections.Generic;
using Dosi.Tracker.Activities.Retention;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;

namespace Dosi.Tracker.Retention;

/// <summary>
/// How the API host runs data retention: the <see cref="CaptureRetentionOptions"/> and <see cref="ActivityRetentionOptions"/>
/// sections are bound whatever else is configured (the default database blob store included), and each worker runs
/// unless the operator switched it off. Both delete only what a workspace owner's retention policy asks for.
/// </summary>
public static class DataRetentionHosting
{
    /// <summary>Binds the retention options from <paramref name="configuration"/>.</summary>
    public static IServiceCollection AddTrackerDataRetention(this IServiceCollection services, IConfiguration configuration)
    {
        services.Configure<CaptureRetentionOptions>(configuration.GetSection(CaptureRetentionOptions.SectionName));
        services.Configure<ActivityRetentionOptions>(configuration.GetSection(ActivityRetentionOptions.SectionName));
        return services;
    }

    /// <summary>The retention workers the host starts with these options.</summary>
    public static IReadOnlyList<Type> WorkerTypes(CaptureRetentionOptions capture, ActivityRetentionOptions activity)
    {
        var workers = new List<Type>(2);
        if (capture.IsEnabled)
        {
            workers.Add(typeof(CaptureRetentionWorker));
        }

        if (activity.IsEnabled)
        {
            workers.Add(typeof(ActivityRetentionWorker));
        }

        return workers;
    }
}
