using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.SaaS;

public class SaaSDomainTests
{
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
}
