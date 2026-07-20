using System;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Activities;

public class ActivityAppService : ApplicationService, IActivityAppService
{
    private readonly IRepository<Activity, Guid> _activityRepository;

    public ActivityAppService(IRepository<Activity, Guid> activityRepository)
    {
        _activityRepository = activityRepository;
    }

    public async Task<ActivityDto> CreateAsync(CreateActivityDto input)
    {
        var existingActivity = await _activityRepository.FirstOrDefaultAsync(
            x => x.ClientActivityId == input.ClientActivityId);
            
        if (existingActivity != null)
        {
            // Idempotent: If agent retries, just return the existing one.
            return MapToDto(existingActivity);
        }

        var activity = new Activity(
            GuidGenerator.Create(),
            CurrentTenant.Id,
            CurrentUser.Id.GetValueOrDefault(), // Fallback to empty GUID if not logged in (e.g. host context error)
            input.ProjectId,
            input.ClientActivityId,
            input.StartedAt,
            input.EndedAt
        )
        {
            Productivity = input.Productivity,
            MouseClicks = input.MouseClicks,
            KeyboardHits = input.KeyboardHits,
            Description = input.Description,
            ActiveWindowsJson = input.ActiveWindows != null ? JsonSerializer.Serialize(input.ActiveWindows) : "[]",
            RunningProgramsJson = input.RunningPrograms != null ? JsonSerializer.Serialize(input.RunningPrograms) : "[]"
        };

        await _activityRepository.InsertAsync(activity);
        
        return MapToDto(activity);
    }

    public async Task<PagedResultDto<ActivityDto>> GetListAsync(PagedAndSortedResultRequestDto input)
    {
        var query = await _activityRepository.GetQueryableAsync();
        
        var totalCount = await AsyncExecuter.CountAsync(query);
        var activities = await AsyncExecuter.ToListAsync(
            query.PageBy(input.SkipCount, input.MaxResultCount)
        );

        return new PagedResultDto<ActivityDto>(
            totalCount,
            activities.Select(MapToDto).ToList()
        );
    }

    private ActivityDto MapToDto(Activity activity)
    {
        return new ActivityDto
        {
            Id = activity.Id,
            UserId = activity.UserId,
            ProjectId = activity.ProjectId,
            ClientActivityId = activity.ClientActivityId,
            StartedAt = activity.StartedAt,
            EndedAt = activity.EndedAt,
            Productivity = activity.Productivity,
            MouseClicks = activity.MouseClicks,
            KeyboardHits = activity.KeyboardHits,
            Description = activity.Description,
            ActiveWindowsJson = activity.ActiveWindowsJson,
            RunningProgramsJson = activity.RunningProgramsJson,
            CreationTime = activity.CreationTime
        };
    }
}
