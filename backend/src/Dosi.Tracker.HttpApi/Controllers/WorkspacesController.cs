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
}

/// <summary>Workspace invitations, addressed by their token.</summary>
[Route("api/app/invitations")]
[WorkspaceNotRequired]
public class InvitationsController : TrackerController
{
    private readonly IWorkspaceAppService _workspaceAppService;

    public InvitationsController(IWorkspaceAppService workspaceAppService)
    {
        _workspaceAppService = workspaceAppService;
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
