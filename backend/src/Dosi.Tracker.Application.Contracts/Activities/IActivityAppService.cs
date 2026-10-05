using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Activities;

public interface IActivityAppService : IApplicationService
{
    Task<ActivityDto> CreateAsync(CreateActivityDto input);

    Task<PagedResultDto<ActivityDto>> GetListAsync(GetActivitiesInput input);

    Task<List<ScreenshotDto>> GetScreenshotsAsync(Guid activityId);

    Task<ScreenshotContentDto> GetScreenshotContentAsync(Guid id);

    /// <summary>Changes the task note of a block (<c>PUT /api/app/activity/{id}/note</c>): allowed for the block's
    /// owner within 30 days of its start, and for callers with the Tracker.Team.Manage permission.</summary>
    Task<ActivityDto> UpdateNoteAsync(Guid id, UpdateActivityNoteDto input);
}
