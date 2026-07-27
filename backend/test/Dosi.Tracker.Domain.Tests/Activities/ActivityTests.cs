using System;
using Shouldly;
using Volo.Abp;
using Xunit;

namespace Dosi.Tracker.Activities;

/* Pure unit tests for the Activity aggregate's construction invariants.
 * These do not need the ABP test module infrastructure. */
public class ActivityTests
{
    private static readonly Guid TestTenantId = Guid.NewGuid();
    private static readonly Guid TestUserId = Guid.NewGuid();
    private static readonly Guid TestProjectId = Guid.NewGuid();

    private static Activity CreateValid(
        Guid? clientActivityId = null,
        DateTime? startedAt = null,
        DateTime? endedAt = null)
    {
        var start = startedAt ?? new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);
        return new Activity(
            Guid.NewGuid(),
            TestTenantId,
            TestUserId,
            TestProjectId,
            clientActivityId ?? Guid.NewGuid(),
            start,
            endedAt ?? start.AddMinutes(10)
        );
    }

    [Fact]
    public void Should_Create_With_Valid_Time_Block()
    {
        var clientActivityId = Guid.NewGuid();
        var start = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);
        var end = start.AddMinutes(10);

        var activity = new Activity(
            Guid.NewGuid(), TestTenantId, TestUserId, TestProjectId, clientActivityId, start, end);

        activity.TenantId.ShouldBe(TestTenantId);
        activity.UserId.ShouldBe(TestUserId);
        activity.ProjectId.ShouldBe(TestProjectId);
        activity.ClientActivityId.ShouldBe(clientActivityId);
        activity.StartedAt.ShouldBe(start);
        activity.EndedAt.ShouldBe(end);
        // Window/program captures default to an empty JSON array, never null or "".
        activity.ActiveWindowsJson.ShouldBe("[]");
        activity.RunningProgramsJson.ShouldBe("[]");
    }

    [Fact]
    public void Should_Reject_Empty_ClientActivityId()
    {
        // Regression guard: the macOS agent currently omits clientActivityId, which would
        // serialize to Guid.Empty and make every later upload dedupe against the first one.
        var exception = Should.Throw<BusinessException>(() => CreateValid(clientActivityId: Guid.Empty));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityClientIdRequired);
        // The offending value is attached under its own key for diagnostics.
        exception.Data["clientActivityId"].ShouldBe(Guid.Empty);
    }

    [Fact]
    public void Should_Reject_EndedAt_Before_StartedAt()
    {
        var start = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);
        var end = start.AddMinutes(-5);

        var exception = Should.Throw<BusinessException>(
            () => CreateValid(startedAt: start, endedAt: end));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTimeRangeInvalid);
        exception.Data["startedAt"].ShouldBe(start);
        exception.Data["endedAt"].ShouldBe(end);
    }

    [Fact]
    public void Should_Reject_Zero_Length_Time_Block()
    {
        var start = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);

        var exception = Should.Throw<BusinessException>(
            () => CreateValid(startedAt: start, endedAt: start));
        exception.Code.ShouldBe(TrackerDomainErrorCodes.ActivityTimeRangeInvalid);
    }
}
