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
}
