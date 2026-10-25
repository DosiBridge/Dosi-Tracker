using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/// <summary>The signed-in person's own account (<see cref="IMyAccountAppService"/>; no <c>X-Workspace-Id</c>).</summary>
[Route("api/app/my-account")]
[Authorize]
[WorkspaceNotRequired]
public class MyAccountController : TrackerController, IMyAccountAppService
{
    private readonly IMyAccountAppService _myAccountAppService;

    public MyAccountController(IMyAccountAppService myAccountAppService)
    {
        _myAccountAppService = myAccountAppService;
    }

    [HttpPost("send-email-confirmation")]
    public Task<EmailConfirmationSentDto> SendEmailConfirmationAsync()
    {
        return _myAccountAppService.SendEmailConfirmationAsync();
    }

    [HttpPost("confirm-email")]
    [AllowAnonymous]
    public Task ConfirmEmailAsync([FromBody] ConfirmEmailDto input)
    {
        return _myAccountAppService.ConfirmEmailAsync(input);
    }

    [HttpGet("deletion-check")]
    public Task<AccountDeletionCheckDto> GetDeletionCheckAsync()
    {
        return _myAccountAppService.GetDeletionCheckAsync();
    }

    [HttpPost("delete")]
    public Task DeleteAsync([FromBody] DeleteMyAccountDto input)
    {
        return _myAccountAppService.DeleteAsync(input);
    }
}
