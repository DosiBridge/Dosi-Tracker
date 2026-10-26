using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/// <summary>The signed-in user's workspaces (no <c>X-Workspace-Id</c> needed).</summary>
[Route("api/app/workspaces")]
[Authorize]
[WorkspaceNotRequired]
public class WorkspacesController : TrackerController
{
    private readonly IWorkspaceAppService _workspaceAppService;

    public WorkspacesController(IWorkspaceAppService workspaceAppService)
    {
        _workspaceAppService = workspaceAppService;
    }

    [HttpGet("mine")]
    public Task<List<MyWorkspaceDto>> GetMineAsync()
    {
        return _workspaceAppService.GetMineAsync();
    }

    [HttpPost]
    public Task<CreatedWorkspaceDto> CreateAsync([FromBody] CreateWorkspaceDto input)
    {
        return _workspaceAppService.CreateAsync(input);
    }

    [HttpPost("{id}/leave")]
    public Task LeaveAsync(Guid id)
    {
        return _workspaceAppService.LeaveAsync(id);
    }

    [HttpPost("{id}/archive")]
    public Task<WorkspaceDto> ArchiveAsync(Guid id)
    {
        return _workspaceAppService.ArchiveAsync(id);
    }

    [HttpPost("{id}/restore")]
    public Task<WorkspaceDto> RestoreAsync(Guid id)
    {
        return _workspaceAppService.RestoreAsync(id);
    }

    [HttpPost("{id}/delete")]
    public Task<DeletedWorkspaceDto> DeleteAsync(Guid id, [FromBody] DeleteWorkspaceDto input)
    {
        return _workspaceAppService.DeleteAsync(id, input);
    }

    [HttpGet("{id}/capture-retention")]
    public Task<WorkspaceCaptureRetentionDto> GetCaptureRetentionAsync(Guid id)
    {
        return _workspaceAppService.GetCaptureRetentionAsync(id);
    }

    [HttpPut("{id}/capture-retention")]
    public Task<WorkspaceCaptureRetentionDto> UpdateCaptureRetentionAsync(Guid id, [FromBody] WorkspaceCaptureRetentionDto input)
    {
        return _workspaceAppService.UpdateCaptureRetentionAsync(id, input);
    }

    [HttpGet("{id}/activity-retention")]
    public Task<WorkspaceActivityRetentionDto> GetActivityRetentionAsync(Guid id)
    {
        return _workspaceAppService.GetActivityRetentionAsync(id);
    }

    [HttpPut("{id}/activity-retention")]
    public Task<WorkspaceActivityRetentionDto> UpdateActivityRetentionAsync(Guid id, [FromBody] WorkspaceActivityRetentionDto input)
    {
        return _workspaceAppService.UpdateActivityRetentionAsync(id, input);
    }
}

/// <summary>Workspace invitations, addressed by their token — or, under <c>mine</c>, the signed-in user's own by id.</summary>
[Route("api/app/invitations")]
[WorkspaceNotRequired]
public class InvitationsController : TrackerController
{
    private readonly IWorkspaceAppService _workspaceAppService;

    public InvitationsController(IWorkspaceAppService workspaceAppService)
    {
        _workspaceAppService = workspaceAppService;
    }

    /// <summary>The caller's pending invitations (a literal segment, so it wins over <c>{token}</c>).</summary>
    [HttpGet("mine")]
    [Authorize]
    public Task<MyInvitationsDto> GetMineAsync()
    {
        return _workspaceAppService.GetMyInvitationsAsync();
    }

    [HttpPost("mine/{id:guid}/accept")]
    [Authorize]
    public Task<AcceptInvitationResultDto> AcceptMineAsync(Guid id)
    {
        return _workspaceAppService.AcceptMyInvitationAsync(id);
    }

    [HttpPost("mine/{id:guid}/decline")]
    [Authorize]
    public Task DeclineMineAsync(Guid id)
    {
        return _workspaceAppService.DeclineMyInvitationAsync(id);
    }

    [HttpGet("{token}")]
    [AllowAnonymous]
    public Task<InvitationInfoDto> GetAsync(string token)
    {
        return _workspaceAppService.GetInvitationAsync(token);
    }

    [HttpPost("{token}/accept")]
    [Authorize]
    public Task<AcceptInvitationResultDto> AcceptAsync(string token)
    {
        return _workspaceAppService.AcceptInvitationAsync(token);
    }

    [HttpPost("{token}/decline")]
    [Authorize]
    public Task DeclineAsync(string token)
    {
        return _workspaceAppService.DeclineInvitationAsync(token);
    }
}
