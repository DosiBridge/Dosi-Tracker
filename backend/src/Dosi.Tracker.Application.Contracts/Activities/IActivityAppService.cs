using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Activities;

public interface IActivityAppService : IApplicationService
{
    Task<ActivityDto> CreateAsync(CreateActivityDto input);
    Task<PagedResultDto<ActivityDto>> GetListAsync(PagedAndSortedResultRequestDto input);
}
