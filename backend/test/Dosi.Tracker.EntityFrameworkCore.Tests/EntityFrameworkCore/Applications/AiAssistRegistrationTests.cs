using System.Net;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.AiAssist.Provider;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp.AspNetCore.ExceptionHandling;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Applications;

/// <summary>
/// AI assist as the modules wire it: off by default (the "no AI" provider, options bound from section "AI" of the
/// configuration the application uses, which in tests has no AI section), and each Tracker:Ai* code with the HTTP
/// status the web app decides by.
/// </summary>
public class AiAssistRegistrationTests : TrackerTestBase<TrackerHttpApiServerTestModule>
{
    [Fact]
    public void Ai_Is_Off_Unless_A_Host_Supplies_A_Provider()
    {
        var provider = GetRequiredService<IAiChatClientProvider>();

        provider.ShouldBeOfType<NullAiChatClientProvider>();
        provider.ShouldBeSameAs(GetRequiredService<IAiChatClientProvider>());
        provider.GetClient(AiFeatureIds.AskId).ShouldBeNull();
    }

    [Fact]
    public void The_Options_Are_Bound_And_Default_To_Off()
    {
        var options = GetRequiredService<IOptions<AiOptions>>().Value;

        options.Enabled.ShouldBeFalse();
        options.OpenAI.AllowedHosts.ShouldBe("api.openai.com");
        options.Quotas.For("Free").MonthlyRequests.ShouldBe(50);
    }

    [Theory]
    [InlineData(TrackerDomainErrorCodes.AiNotConfigured, HttpStatusCode.ServiceUnavailable)]
    [InlineData(TrackerDomainErrorCodes.AiPlatformDisabled, HttpStatusCode.ServiceUnavailable)]
    [InlineData(TrackerDomainErrorCodes.AiDisabled, HttpStatusCode.Conflict)]
    [InlineData(TrackerDomainErrorCodes.AiFeatureDisabled, HttpStatusCode.Forbidden)]
    [InlineData(TrackerDomainErrorCodes.AiQuotaExceeded, HttpStatusCode.TooManyRequests)]
    [InlineData(TrackerDomainErrorCodes.AiRateLimited, HttpStatusCode.TooManyRequests)]
    [InlineData(TrackerDomainErrorCodes.AiProviderUnavailable, HttpStatusCode.ServiceUnavailable)]
    [InlineData(TrackerDomainErrorCodes.AiOutputInvalid, HttpStatusCode.BadGateway)]
    [InlineData(TrackerDomainErrorCodes.AiRangeTooLarge, HttpStatusCode.BadRequest)]
    [InlineData(TrackerDomainErrorCodes.AiDisclosureNotAccepted, HttpStatusCode.BadRequest)]
    [InlineData(TrackerDomainErrorCodes.AiConversationExpired, HttpStatusCode.Conflict)]
    public void Each_Ai_Error_Code_Has_Its_Status(string code, HttpStatusCode status)
    {
        GetRequiredService<IOptions<AbpExceptionHttpStatusCodeOptions>>().Value
            .ErrorCodeToHttpStatusCodeMappings[code].ShouldBe(status);
    }
}
