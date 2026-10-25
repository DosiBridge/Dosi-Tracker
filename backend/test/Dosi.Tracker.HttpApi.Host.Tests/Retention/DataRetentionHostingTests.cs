using System;
using System.Collections.Generic;
using Dosi.Tracker.Activities.Retention;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Retention;

/// <summary>
/// Retention must run on every deployment, the default database blob store included: the options bind without any S3
/// setting, both workers start by default, and only the operator switches stop them.
/// </summary>
public class DataRetentionHostingTests
{
    private static ServiceProvider Build(Dictionary<string, string?> settings)
    {
        var configuration = new ConfigurationBuilder().AddInMemoryCollection(settings).Build();
        return new ServiceCollection().AddOptions().AddTrackerDataRetention(configuration).BuildServiceProvider();
    }

    [Fact]
    public void Without_Any_Configuration_Both_Workers_Should_Run()
    {
        using var services = Build(new Dictionary<string, string?>());

        var capture = services.GetRequiredService<IOptions<CaptureRetentionOptions>>().Value;
        var activity = services.GetRequiredService<IOptions<ActivityRetentionOptions>>().Value;

        capture.IsEnabled.ShouldBeTrue();
        capture.PeriodHours.ShouldBe(24);
        activity.IsEnabled.ShouldBeTrue();
        DataRetentionHosting.WorkerTypes(capture, activity)
            .ShouldBe(new[] { typeof(CaptureRetentionWorker), typeof(ActivityRetentionWorker) });
    }

    [Fact]
    public void The_Sections_Should_Bind_Without_Any_S3_Setting()
    {
        using var services = Build(new Dictionary<string, string?>
        {
            ["S3:AccessKeyId"] = "",
            ["CaptureRetention:PeriodHours"] = "6",
            ["CaptureRetention:BatchSize"] = "50",
            ["ActivityRetention:PeriodHours"] = "12",
            ["ActivityRetention:MaxPagesPerWorkspace"] = "3"
        });

        var capture = services.GetRequiredService<IOptions<CaptureRetentionOptions>>().Value;
        capture.PeriodHours.ShouldBe(6);
        capture.BatchSize.ShouldBe(50);

        var activity = services.GetRequiredService<IOptions<ActivityRetentionOptions>>().Value;
        activity.PeriodHours.ShouldBe(12);
        activity.MaxPagesPerWorkspace.ShouldBe(3);
    }

    [Theory]
    [InlineData(false, true, typeof(ActivityRetentionWorker))]
    [InlineData(true, false, typeof(CaptureRetentionWorker))]
    public void An_Operator_Should_Be_Able_To_Switch_Each_Worker_Off(bool captureEnabled, bool activityEnabled, Type remaining)
    {
        using var services = Build(new Dictionary<string, string?>
        {
            ["CaptureRetention:IsEnabled"] = captureEnabled.ToString(),
            ["ActivityRetention:IsEnabled"] = activityEnabled.ToString()
        });

        DataRetentionHosting.WorkerTypes(
                services.GetRequiredService<IOptions<CaptureRetentionOptions>>().Value,
                services.GetRequiredService<IOptions<ActivityRetentionOptions>>().Value)
            .ShouldBe(new[] { remaining });
    }
}
