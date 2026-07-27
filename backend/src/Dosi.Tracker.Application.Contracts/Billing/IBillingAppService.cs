using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Billing;

public interface IBillingAppService : IApplicationService
{
    Task<PagedResultDto<InvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input);
}
