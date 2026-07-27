using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Billing;

public class BillingDomainTests
{
    [Fact]
    public void New_Invoice_Should_Start_Pending()
    {
        var invoice = new Invoice(Guid.NewGuid(), Guid.NewGuid(), 120.50m, DateTime.UtcNow.AddDays(14));

        invoice.Status.ShouldBe("pending");
        invoice.Amount.ShouldBe(120.50m);
    }
}
