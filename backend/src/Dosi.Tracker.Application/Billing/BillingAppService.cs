using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Permissions;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Billing;

[Authorize(TrackerPermissions.Billing.Default)]
public class BillingAppService : TrackerAppService, IBillingAppService
{
    private readonly IRepository<Invoice, Guid> _invoiceRepository;

    public BillingAppService(IRepository<Invoice, Guid> invoiceRepository)
    {
        _invoiceRepository = invoiceRepository;
    }

    public async Task<PagedResultDto<InvoiceDto>> GetInvoicesAsync(PagedAndSortedResultRequestDto input)
    {
        var query = await _invoiceRepository.GetQueryableAsync();
        var totalCount = await AsyncExecuter.CountAsync(query);
        var items = await AsyncExecuter.ToListAsync(
            query
                .OrderByDescending(i => i.DueDate) // Deterministic order; Skip/Take without OrderBy is undefined.
                .PageBy(input.SkipCount, input.MaxResultCount));

        return new PagedResultDto<InvoiceDto>(
            totalCount,
            ObjectMapper.Map<List<Invoice>, List<InvoiceDto>>(items)
        );
    }
}
