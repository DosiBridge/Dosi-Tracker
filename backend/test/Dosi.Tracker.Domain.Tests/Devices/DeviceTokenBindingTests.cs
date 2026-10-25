using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Devices;

public class DeviceTokenBindingTests
{
    private const string Laptop = "4c1f0f5e-8a43-4b6e-9b1d-2f6f1c0a9e11";
    private const string Desktop = "7e2a6b90-1d3c-4f85-a7c4-5b8e9d0f1a22";

    [Theory]
    [InlineData(null, null, DeviceTokenBinding.Outcome.Unbound, null)]
    [InlineData("  ", null, DeviceTokenBinding.Outcome.Unbound, null)]
    [InlineData(Laptop, null, DeviceTokenBinding.Outcome.Bound, Laptop)] // password grant, or first refresh of a legacy token
    [InlineData(" " + Laptop + " ", null, DeviceTokenBinding.Outcome.Bound, Laptop)]
    [InlineData(null, Laptop, DeviceTokenBinding.Outcome.Bound, Laptop)] // refresh that leaves device_id out
    [InlineData(Laptop, Laptop, DeviceTokenBinding.Outcome.Bound, Laptop)]
    [InlineData(Desktop, Laptop, DeviceTokenBinding.Outcome.DeviceMismatch, Laptop)]
    [InlineData("laptop", null, DeviceTokenBinding.Outcome.InvalidDeviceId, null)]
    [InlineData("00000000-0000-0000-0000-000000000000", Laptop, DeviceTokenBinding.Outcome.InvalidDeviceId, null)]
    [InlineData(null, "junk", DeviceTokenBinding.Outcome.Unbound, null)] // an unreadable claim binds nothing
    public void Decide_Should_Bind_Keep_Or_Refuse_The_Device(string? requested, string? bound, DeviceTokenBinding.Outcome outcome, string? device)
    {
        var decision = DeviceTokenBinding.Decide(requested, bound);

        decision.Outcome.ShouldBe(outcome);
        decision.DeviceId.ShouldBe(device == null ? null : Guid.Parse(device));
    }

    [Fact]
    public void Parse_And_Format_Should_Round_Trip_Only_Non_Empty_Ids()
    {
        var id = Guid.Parse(Laptop);

        DeviceTokenBinding.Parse(DeviceTokenBinding.Format(id)).ShouldBe(id);
        DeviceTokenBinding.Format(id).ShouldBe(Laptop);
        DeviceTokenBinding.Parse(Laptop.ToUpperInvariant()).ShouldBe(id);
        DeviceTokenBinding.Parse(Guid.Empty.ToString()).ShouldBeNull();
        DeviceTokenBinding.Parse(null).ShouldBeNull();
        DeviceTokenBinding.Parse("nope").ShouldBeNull();
    }
}
