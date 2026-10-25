using System.Linq;
using System.Reflection;
using Dosi.Tracker.Localization;
using Microsoft.Extensions.Localization;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp.AspNetCore.ExceptionHandling;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>
/// Every Tracker error code has an English message and its own HTTP status: ABP answers an unmapped business error
/// with 403, which agents and the web app would read as a missing permission, and a code without a message shows the
/// bare code to people.
/// </summary>
public class ErrorCodeCatalogTests : TrackerTestBase<TrackerHttpApiServerTestModule>
{
    private static readonly string[] Codes = typeof(TrackerDomainErrorCodes)
        .GetFields(BindingFlags.Public | BindingFlags.Static)
        .Where(f => f.IsLiteral && f.FieldType == typeof(string))
        .Select(f => (string)f.GetRawConstantValue()!)
        .ToArray();

    [Fact]
    public void Every_Error_Code_Has_An_English_Message()
    {
        Codes.ShouldContain(TrackerDomainErrorCodes.ActivityDeleted);
        var localizer = GetRequiredService<IStringLocalizer<TrackerResource>>();
        using (CultureHelperScope.English())
        {
            localizer["Tracker:NoSuchCode"].ResourceNotFound.ShouldBeTrue();
            Codes.Where(code => localizer[code].ResourceNotFound).ShouldBeEmpty();
        }
    }

    [Fact]
    public void Every_Error_Code_Has_An_Http_Status()
    {
        var mappings = GetRequiredService<IOptions<AbpExceptionHttpStatusCodeOptions>>().Value.ErrorCodeToHttpStatusCodeMappings;

        Codes.Where(code => !mappings.ContainsKey(code)).ShouldBeEmpty();
    }

    private static class CultureHelperScope
    {
        public static System.IDisposable English() => Volo.Abp.Localization.CultureHelper.Use("en");
    }
}
