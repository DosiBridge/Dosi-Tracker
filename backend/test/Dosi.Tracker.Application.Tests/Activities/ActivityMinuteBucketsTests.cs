using System;
using System.Collections.Generic;
using System.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Activities;

/* Pure unit tests for the minute buckets of a block: the same cut as the agents' activity % (input.rs). */
public class ActivityMinuteBucketsTests
{
    [Theory]
    [InlineData(0, 0)]
    [InlineData(1, 1)]
    [InlineData(89, 1)]
    [InlineData(90, 2)]   // rounded to the nearest minute, half up
    [InlineData(149, 2)]
    [InlineData(150, 3)]
    [InlineData(600, 10)]
    [InlineData(629, 10)]
    [InlineData(631, 11)]
    [InlineData(7200, 120)]
    public void Count_Should_Round_The_Block_Length_To_The_Nearest_Minute(int spanSeconds, int expected)
    {
        ActivityMinuteBuckets.Count(spanSeconds).ShouldBe(expected);
    }

    [Theory]
    [InlineData(0, 629, 60)]
    [InlineData(8, 629, 60)]
    [InlineData(9, 629, 89)]  // the last bucket absorbs the remainder
    [InlineData(10, 629, 0)]  // past the block
    [InlineData(-1, 629, 0)]
    [InlineData(10, 631, 31)]
    [InlineData(0, 45, 45)]   // a block under 90 s is one bucket
    [InlineData(0, 0, 0)]
    public void LengthSeconds_Should_Give_Each_Bucket_Its_Share_Of_The_Block(int minute, int spanSeconds, int expected)
    {
        ActivityMinuteBuckets.LengthSeconds(minute, spanSeconds).ShouldBe(expected);
    }

    [Fact]
    public void Buckets_Should_Add_Up_To_The_Block()
    {
        foreach (var span in new[] { 1, 59, 60, 61, 89, 90, 119, 599, 600, 629, 630, 631, 3599, 7200 })
        {
            Enumerable.Range(0, ActivityMinuteBuckets.Count(span))
                .Sum(minute => ActivityMinuteBuckets.LengthSeconds(minute, span))
                .ShouldBe(span);
        }
    }

    [Fact]
    public void SpanSeconds_Should_Round_The_Block_To_Whole_Seconds()
    {
        ActivityMinuteBuckets.SpanSeconds(TimeSpan.FromSeconds(629.4)).ShouldBe(629);
        ActivityMinuteBuckets.SpanSeconds(TimeSpan.FromSeconds(629.5)).ShouldBe(630);
        ActivityMinuteBuckets.SpanSeconds(TimeSpan.FromSeconds(-3)).ShouldBe(0);
    }

    [Fact]
    public void IdleSeconds_Should_Sum_The_Inactive_Buckets()
    {
        var timeline = Enumerable.Range(0, 10)
            .Select(i => new ActivityMinuteDto { Minute = i, Active = i is not (3 or 9) })
            .ToList();

        ActivityMinuteBuckets.IdleSeconds(timeline, TimeSpan.FromSeconds(629)).ShouldBe(60 + 89);
    }

    [Fact]
    public void IdleSeconds_Should_Count_Only_Reported_Buckets_Inside_The_Block_And_Each_Once()
    {
        var timeline = new List<ActivityMinuteDto>
        {
            new() { Minute = 1, Active = false },
            new() { Minute = 1, Active = true },   // duplicate: the first entry counts
            new() { Minute = 2, Active = true },
            new() { Minute = 2, Active = false },  // duplicate: ignored
            new() { Minute = 12, Active = false }, // outside the block
            null!
        };

        ActivityMinuteBuckets.IdleSeconds(timeline, TimeSpan.FromMinutes(10)).ShouldBe(60);
    }

    [Fact]
    public void IdleSeconds_Should_Be_The_Whole_Block_When_Nothing_Was_Active()
    {
        var timeline = Enumerable.Range(0, 2).Select(i => new ActivityMinuteDto { Minute = i }).ToList();

        ActivityMinuteBuckets.IdleSeconds(timeline, TimeSpan.FromSeconds(100)).ShouldBe(100);
        ActivityMinuteBuckets.IdleSeconds(new List<ActivityMinuteDto>(), TimeSpan.FromMinutes(5)).ShouldBe(0);
    }
}
