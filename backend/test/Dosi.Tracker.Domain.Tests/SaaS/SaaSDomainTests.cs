using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.SaaS;

public class SaaSDomainTests
{
    private static readonly DateTime Now = new(2026, 10, 5, 12, 0, 0, DateTimeKind.Utc);

    [Fact]
    public void New_Subscription_Should_Start_Trialing()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());

        subscription.Status.ShouldBe("trialing");
        subscription.TrialEndsAt.ShouldBeNull();
    }

    [Fact]
    public void Plan_Should_Keep_Pricing_Fields()
    {
        var plan = new Plan(Guid.NewGuid(), "Starter", 6m, 25, 14);

        plan.Name.ShouldBe("Starter");
        plan.PricePerUser.ShouldBe(6m);
        plan.MaxSeats.ShouldBe(25);
        plan.TrialDays.ShouldBe(14);
    }

    [Theory]
    [InlineData(0, true)]
    [InlineData(-1, true)]
    [InlineData(0.01, false)]
    [InlineData(6, false)]
    public void A_Plan_Without_A_Price_Should_Be_Free(double pricePerUser, bool free)
    {
        new Plan(Guid.NewGuid(), "Plan", (decimal)pricePerUser, 3, 0).IsFree().ShouldBe(free);
    }

    [Fact]
    public void A_Trial_Should_Be_Over_Only_After_Its_End()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
        subscription.StartTrial(Now);

        subscription.Status.ShouldBe(Subscription.TrialingStatus);
        subscription.IsTrialOver(Now.AddSeconds(-1)).ShouldBeFalse();
        subscription.IsTrialOver(Now).ShouldBeFalse();
        subscription.IsTrialOver(Now.AddSeconds(1)).ShouldBeTrue();
    }

    [Fact]
    public void A_Trial_Without_An_End_Or_Another_Status_Should_Never_Be_Over()
    {
        var withoutEnd = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
        withoutEnd.IsTrialOver(Now.AddYears(5)).ShouldBeFalse();

        var pastDue = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid()) { TrialEndsAt = Now.AddDays(-30) };
        pastDue.MarkPastDue();
        pastDue.IsTrialOver(Now).ShouldBeFalse(); // only a running trial can be "over"
    }

    [Fact]
    public void Activate_Should_End_The_Trial()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
        subscription.StartTrial(Now.AddDays(10));

        subscription.Activate();

        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        subscription.TrialEndsAt.ShouldBeNull();
    }

    [Fact]
    public void Status_Transitions_Should_Set_Their_Status()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());

        subscription.MarkPastDue();
        subscription.Status.ShouldBe(Subscription.PastDueStatus);

        subscription.Suspend();
        subscription.Status.ShouldBe(Subscription.SuspendedStatus);

        subscription.Cancel();
        subscription.Status.ShouldBe(Subscription.CanceledStatus);
    }

    [Fact]
    public void Changing_To_A_Free_Plan_Should_Activate()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
        subscription.StartTrial(Now.AddDays(3));
        var free = new Plan(Guid.NewGuid(), Plan.FreePlanName, 0m, 3, 0);

        subscription.ChangePlan(free);

        subscription.PlanId.ShouldBe(free.Id);
        subscription.Status.ShouldBe(Subscription.ActiveStatus);
        subscription.TrialEndsAt.ShouldBeNull();
    }

    [Theory]
    [InlineData(Subscription.TrialingStatus)]
    [InlineData(Subscription.ActiveStatus)]
    [InlineData(Subscription.PastDueStatus)]
    [InlineData(Subscription.SuspendedStatus)]
    public void Changing_To_A_Paid_Plan_Should_Keep_The_Status(string status)
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid())
        {
            Status = status,
            TrialEndsAt = Now.AddDays(4)
        };
        var business = new Plan(Guid.NewGuid(), "Business", 12m, 100, 14);

        subscription.ChangePlan(business);

        subscription.PlanId.ShouldBe(business.Id);
        subscription.Status.ShouldBe(status);
        subscription.TrialEndsAt.ShouldBe(Now.AddDays(4));
    }

    [Fact]
    public void The_Trial_Reminder_Should_Be_Remembered()
    {
        var subscription = new Subscription(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid());
        subscription.IsTrialReminderSent().ShouldBeFalse();

        subscription.MarkTrialReminderSent();

        subscription.IsTrialReminderSent().ShouldBeTrue();
    }
}
