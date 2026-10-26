using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

public class AiConcurrencyGateTests
{
    private static readonly Guid WorkspaceA = Guid.NewGuid();
    private static readonly Guid WorkspaceB = Guid.NewGuid();

    private static AiConcurrencyGate Gate(int global, int perWorkspace) =>
        new(Options.Create(new AiOptions { Concurrency = new AiConcurrencyOptions { Global = global, PerWorkspace = perWorkspace } }),
            TimeSpan.FromMilliseconds(50));

    private static async Task<BusinessException> BusyAsync(AiConcurrencyGate gate, Guid workspaceId)
    {
        var refused = await Should.ThrowAsync<BusinessException>(() => gate.AcquireAsync(workspaceId, AiFeatureIds.AskId));
        refused.Code.ShouldBe(TrackerDomainErrorCodes.AiRateLimited);
        refused.Data["reason"].ShouldBe(AiErrors.BusyReason);
        refused.Data["retryAfterSeconds"].ShouldBe(AiConcurrencyGate.BusyRetryAfterSeconds);
        refused.Data["featureId"].ShouldBe(AiFeatureIds.AskId);
        return refused;
    }

    [Fact]
    public async Task A_Workspace_Should_Not_Take_More_Than_Its_Share()
    {
        using var gate = Gate(global: 4, perWorkspace: 2);
        using var first = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);
        using var second = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);

        await BusyAsync(gate, WorkspaceA);

        using var other = await gate.AcquireAsync(WorkspaceB, AiFeatureIds.AskId);
    }

    [Fact]
    public async Task The_Instance_Should_Not_Take_More_Than_Its_Global_Limit()
    {
        using var gate = Gate(global: 2, perWorkspace: 2);
        using var first = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);
        using var second = await gate.AcquireAsync(WorkspaceB, AiFeatureIds.AskId);

        await BusyAsync(gate, Guid.NewGuid());
    }

    [Fact]
    public async Task A_Refused_Global_Wait_Should_Give_The_Workspace_Slot_Back()
    {
        using var gate = Gate(global: 1, perWorkspace: 1);
        var held = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);

        await BusyAsync(gate, WorkspaceB);
        held.Dispose();

        using var again = await gate.AcquireAsync(WorkspaceB, AiFeatureIds.AskId);
    }

    [Fact]
    public async Task Releasing_A_Slot_Twice_Should_Release_It_Once()
    {
        using var gate = Gate(global: 1, perWorkspace: 1);
        var slot = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);
        slot.Dispose();
        slot.Dispose();

        using var again = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);
        await BusyAsync(gate, WorkspaceA);
    }

    [Fact]
    public async Task A_Cancelled_Wait_Should_Propagate_The_Cancellation()
    {
        using var gate = Gate(global: 1, perWorkspace: 1);
        using var held = await gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId);
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();

        await Should.ThrowAsync<OperationCanceledException>(() => gate.AcquireAsync(WorkspaceA, AiFeatureIds.AskId, cancelled.Token));
    }

    [Fact]
    public void Limits_Should_Be_At_Least_One_And_A_Workspace_Never_Above_The_Instance()
    {
        using var gate = Gate(global: 0, perWorkspace: 9);

        gate.GlobalLimit.ShouldBe(1);
        gate.PerWorkspaceLimit.ShouldBe(1);
        gate.WaitTimeout.ShouldBe(TimeSpan.FromMilliseconds(50));
        new AiConcurrencyGate(Options.Create(new AiOptions())).WaitTimeout.ShouldBe(AiConcurrencyGate.DefaultWaitTimeout);
    }
}
