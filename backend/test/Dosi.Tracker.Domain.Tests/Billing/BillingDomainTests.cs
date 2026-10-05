using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Billing;

public class BillingDomainTests
{
    private static readonly DateTime Now = new(2026, 10, 5, 12, 0, 0, DateTimeKind.Utc);

    [Fact]
    public void New_Invoice_Should_Start_Pending()
    {
        var invoice = new Invoice(Guid.NewGuid(), Guid.NewGuid(), 120.50m, DateTime.UtcNow.AddDays(14));

        invoice.Status.ShouldBe("pending");
        invoice.Amount.ShouldBe(120.50m);
        invoice.BillingPeriodStart.ShouldBeNull();
    }

    [Theory]
    [InlineData("2026-10-01T00:00:00", "2026-10-01T00:00:00")]
    [InlineData("2026-10-31T23:59:59", "2026-10-01T00:00:00")]
    [InlineData("2026-02-15T08:30:00", "2026-02-01T00:00:00")]
    [InlineData("2026-12-31T23:59:59", "2026-12-01T00:00:00")]
    public void The_Billing_Period_Should_Be_The_Utc_Calendar_Month(string moment, string expectedStart)
    {
        var start = Invoice.GetBillingPeriodStart(DateTime.Parse(moment));

        start.ShouldBe(DateTime.Parse(expectedStart));
        start.Kind.ShouldBe(DateTimeKind.Utc);
    }

    [Fact]
    public void An_Unpaid_Invoice_Should_Be_Overdue_Only_After_Its_Due_Date()
    {
        var invoice = new Invoice(Guid.NewGuid(), Guid.NewGuid(), 12m, Now);

        invoice.IsOverdue(Now).ShouldBeFalse();
        invoice.IsOverdue(Now.AddSeconds(1)).ShouldBeTrue();

        invoice.Status = Invoice.FailedStatus; // a failed payment is still unpaid
        invoice.IsOverdue(Now.AddDays(1)).ShouldBeTrue();
    }

    [Fact]
    public void A_Paid_Invoice_Should_Never_Be_Overdue()
    {
        var invoice = new Invoice(Guid.NewGuid(), Guid.NewGuid(), 12m, Now);
        invoice.IsPaid().ShouldBeFalse();

        invoice.MarkPaid();

        invoice.IsPaid().ShouldBeTrue();
        invoice.Status.ShouldBe(Invoice.PaidStatus);
        invoice.IsOverdue(Now.AddYears(1)).ShouldBeFalse();
    }

    [Fact]
    public void The_Overdue_Notice_Should_Be_Remembered()
    {
        var invoice = new Invoice(Guid.NewGuid(), Guid.NewGuid(), 12m, Now);
        invoice.IsOverdueNoticeSent().ShouldBeFalse();

        invoice.MarkOverdueNoticeSent();

        invoice.IsOverdueNoticeSent().ShouldBeTrue();
    }
}
