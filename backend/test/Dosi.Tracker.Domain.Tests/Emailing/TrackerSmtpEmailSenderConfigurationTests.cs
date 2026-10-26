using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using NSubstitute;
using Shouldly;
using Volo.Abp.Emailing;
using Volo.Abp.SettingManagement;
using Volo.Abp.Settings;
using Xunit;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// SMTP is "configured" only when a host was given explicitly — in configuration or as a platform-wide setting, never by
/// ABP's built-in 127.0.0.1 default. The plain-text password from the environment wins over the encrypted setting, and
/// a configured user name is always used (ABP's default would use the process's own Windows credentials instead).
/// </summary>
public class TrackerSmtpEmailSenderConfigurationTests
{
    private readonly ISettingProvider _settings = Substitute.For<ISettingProvider>();
    private readonly ISettingManager _settingManager = Substitute.For<ISettingManager>();

    private TrackerSmtpEmailSenderConfiguration NewConfiguration(Dictionary<string, string?>? values = null)
    {
        var configuration = new ConfigurationBuilder().AddInMemoryCollection(values ?? new Dictionary<string, string?>()).Build();
        return new TrackerSmtpEmailSenderConfiguration(_settings, _settingManager, configuration);
    }

    private void Setting(string name, string? value) => _settings.GetOrNullAsync(name).Returns(value);

    [Fact]
    public async Task Abps_Default_Host_Alone_Is_Not_A_Configured_Server()
    {
        Setting(EmailSettingNames.Smtp.Host, "127.0.0.1"); // ABP's default, from the default value provider

        (await NewConfiguration().IsSmtpConfiguredAsync()).ShouldBeFalse();
        (await NewConfiguration().CanDeliverAsync()).ShouldBeFalse();
    }

    [Fact]
    public async Task A_Host_From_Configuration_Or_The_Global_Setting_Store_Is()
    {
        (await NewConfiguration(new Dictionary<string, string?>
        {
            [TrackerSmtpEmailSenderConfiguration.HostConfigurationKey] = "smtp.example.com"
        }).IsSmtpConfiguredAsync()).ShouldBeTrue();

        _settingManager.GetOrNullAsync(EmailSettingNames.Smtp.Host, GlobalSettingValueProvider.ProviderName, null, false)
            .Returns("smtp.example.com");
        (await NewConfiguration().IsSmtpConfiguredAsync()).ShouldBeTrue();
    }

    [Fact]
    public async Task The_Plain_Password_From_The_Environment_Wins_Over_The_Encrypted_Setting()
    {
        Setting(EmailSettingNames.Smtp.Password, "decrypted-setting");

        (await NewConfiguration().GetPasswordAsync()).ShouldBe("decrypted-setting");
        (await NewConfiguration(new Dictionary<string, string?>
        {
            [TrackerSmtpEmailSenderConfiguration.PlainPasswordKey] = "from-environment"
        }).GetPasswordAsync()).ShouldBe("from-environment");
    }

    [Fact]
    public async Task A_Configured_User_Name_Is_Always_Used()
    {
        Setting(EmailSettingNames.Smtp.UseDefaultCredentials, "true");
        Setting(EmailSettingNames.Smtp.UserName, null);
        (await NewConfiguration().GetUseDefaultCredentialsAsync()).ShouldBeTrue();
        (await NewConfiguration().GetUserNameAsync()).ShouldBe(string.Empty);

        Setting(EmailSettingNames.Smtp.UserName, "mailer");
        (await NewConfiguration().GetUseDefaultCredentialsAsync()).ShouldBeFalse();
    }

    [Fact]
    public async Task The_Server_Settings_Are_Read_And_A_Missing_Required_One_Is_Reported()
    {
        Setting(EmailSettingNames.Smtp.Host, " smtp.example.com ");
        Setting(EmailSettingNames.Smtp.Port, "587");
        Setting(EmailSettingNames.Smtp.EnableSsl, "true");
        Setting(EmailSettingNames.Smtp.Domain, "corp");
        Setting(EmailSettingNames.DefaultFromAddress, "noreply@example.com");
        Setting(EmailSettingNames.DefaultFromDisplayName, "Dosi Tracker");
        var configuration = NewConfiguration();

        (await configuration.GetHostAsync()).ShouldBe("smtp.example.com");
        (await configuration.GetPortAsync()).ShouldBe(587);
        (await configuration.GetEnableSslAsync()).ShouldBeTrue();
        (await configuration.GetDomainAsync()).ShouldBe("corp");
        (await configuration.GetDefaultFromAddressAsync()).ShouldBe("noreply@example.com");
        (await configuration.GetDefaultFromDisplayNameAsync()).ShouldBe("Dosi Tracker");

        Setting(EmailSettingNames.DefaultFromAddress, " ");
        (await Should.ThrowAsync<InvalidOperationException>(() => configuration.GetDefaultFromAddressAsync()))
            .Message.ShouldContain(EmailSettingNames.DefaultFromAddress);
    }
}
