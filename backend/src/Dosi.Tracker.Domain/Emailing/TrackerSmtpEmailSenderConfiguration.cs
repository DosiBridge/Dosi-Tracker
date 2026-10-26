using System;
using System.Globalization;
using System.Threading.Tasks;
using Microsoft.Extensions.Configuration;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Emailing;
using Volo.Abp.Emailing.Smtp;
using Volo.Abp.SettingManagement;
using Volo.Abp.Settings;

namespace Dosi.Tracker.Emailing;

/// <summary>
/// The SMTP settings outgoing email uses. They are ABP's mailing settings (<see cref="EmailSettingNames"/>), read from
/// configuration — <c>Settings:Abp.Mailing.Smtp.Host</c>, <c>…Port</c>, <c>…UserName</c>, <c>…EnableSsl</c>,
/// <c>Settings:Abp.Mailing.DefaultFromAddress</c> and <c>…DefaultFromDisplayName</c>, as environment variables
/// <c>Settings__Abp.Mailing.Smtp.Host</c> and so on — or from the platform-wide setting store.
/// <para>The password comes either from the setting <c>Abp.Mailing.Smtp.Password</c>, which holds it encrypted with the
/// deployment's StringEncryption passphrase (the setting store encrypts it when it is saved there), or in plain text from
/// <see cref="PlainPasswordKey"/>, which must come from the environment or a secret store: the production start-up guard
/// refuses it from the tracked appsettings.json.</para>
/// <para>Email is sent only once a host is configured explicitly (<see cref="IsSmtpConfiguredAsync"/>): ABP's built-in
/// default host (127.0.0.1) does not count, so a deployment without SMTP logs that an email was not sent instead of
/// failing to reach a mail server that is not there.</para>
/// </summary>
[Dependency(ReplaceServices = true)]
[ExposeServices(
    typeof(ISmtpEmailSenderConfiguration),
    typeof(IEmailSenderConfiguration),
    typeof(IEmailDeliveryChecker),
    typeof(TrackerSmtpEmailSenderConfiguration))]
public class TrackerSmtpEmailSenderConfiguration : ISmtpEmailSenderConfiguration, IEmailDeliveryChecker, ITransientDependency
{
    /// <summary>Plain-text SMTP password (environment variable <c>Email__SmtpPassword</c>); wins over the encrypted
    /// setting.</summary>
    public const string PlainPasswordKey = "Email:SmtpPassword";

    /// <summary>The configuration key ABP reads the SMTP host setting from.</summary>
    public const string HostConfigurationKey = "Settings:" + EmailSettingNames.Smtp.Host;

    private readonly ISettingProvider _settingProvider;
    private readonly ISettingManager _settingManager;
    private readonly IConfiguration _configuration;

    public TrackerSmtpEmailSenderConfiguration(
        ISettingProvider settingProvider,
        ISettingManager settingManager,
        IConfiguration configuration)
    {
        _settingProvider = settingProvider;
        _settingManager = settingManager;
        _configuration = configuration;
    }

    public virtual Task<bool> CanDeliverAsync() => IsSmtpConfiguredAsync();

    /// <summary>True when an SMTP host is configured explicitly: in configuration, or as a platform-wide setting.</summary>
    public virtual async Task<bool> IsSmtpConfiguredAsync()
    {
        if (!string.IsNullOrWhiteSpace(_configuration[HostConfigurationKey]))
        {
            return true;
        }

        return !string.IsNullOrWhiteSpace(await _settingManager.GetOrNullGlobalAsync(EmailSettingNames.Smtp.Host, fallback: false));
    }

    public virtual Task<string> GetHostAsync() => GetRequiredAsync(EmailSettingNames.Smtp.Host);

    public virtual async Task<int> GetPortAsync() => int.Parse(await GetRequiredAsync(EmailSettingNames.Smtp.Port), CultureInfo.InvariantCulture);

    public virtual async Task<string> GetUserNameAsync() =>
        await _settingProvider.GetOrNullAsync(EmailSettingNames.Smtp.UserName) ?? string.Empty;

    public virtual async Task<string> GetPasswordAsync()
    {
        var plain = _configuration[PlainPasswordKey];
        if (!string.IsNullOrEmpty(plain))
        {
            return plain;
        }

        return await _settingProvider.GetOrNullAsync(EmailSettingNames.Smtp.Password) ?? string.Empty;
    }

    public virtual Task<string?> GetDomainAsync() => _settingProvider.GetOrNullAsync(EmailSettingNames.Smtp.Domain);

    public virtual async Task<bool> GetEnableSslAsync() =>
        bool.Parse(await GetRequiredAsync(EmailSettingNames.Smtp.EnableSsl));

    /// <summary>The account given by user name and password is used whenever a user name is configured: ABP's default
    /// (use the process's own Windows credentials) would otherwise silently ignore it.</summary>
    public virtual async Task<bool> GetUseDefaultCredentialsAsync()
    {
        if (!string.IsNullOrWhiteSpace(await GetUserNameAsync()))
        {
            return false;
        }

        return bool.Parse(await GetRequiredAsync(EmailSettingNames.Smtp.UseDefaultCredentials));
    }

    public virtual Task<string> GetDefaultFromAddressAsync() => GetRequiredAsync(EmailSettingNames.DefaultFromAddress);

    public virtual Task<string> GetDefaultFromDisplayNameAsync() => GetRequiredAsync(EmailSettingNames.DefaultFromDisplayName);

    private async Task<string> GetRequiredAsync(string name)
    {
        var value = await _settingProvider.GetOrNullAsync(name);
        if (string.IsNullOrWhiteSpace(value))
        {
            throw new InvalidOperationException($"The email setting '{name}' is empty.");
        }

        return value.Trim();
    }
}

/// <summary>Whether this deployment can deliver email at all (SMTP is configured). Callers report it, so a person is
/// told "we emailed it" only when that is true, and a link to copy otherwise.</summary>
public interface IEmailDeliveryChecker
{
    Task<bool> CanDeliverAsync();
}
