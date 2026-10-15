using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Volo.Abp.DependencyInjection;
using Volo.Abp.SettingManagement;
using Volo.Abp.Settings;

namespace Dosi.Tracker.Settings;

/// <summary>Reads workspace-level setting values ("W", keyed by the current workspace's id).</summary>
public class WorkspaceSettingValueProvider : SettingValueProvider, ITransientDependency
{
    public const string ProviderName = "W";

    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceSettingValueProvider(ISettingStore settingStore, ICurrentWorkspace currentWorkspace)
        : base(settingStore)
    {
        _currentWorkspace = currentWorkspace;
    }

    public override string Name => ProviderName;

    public override async Task<string?> GetOrNullAsync(SettingDefinition setting)
    {
        return _currentWorkspace.Id == null
            ? null
            : await SettingStore.GetOrNullAsync(setting.Name, Name, _currentWorkspace.Id.Value.ToString());
    }

    public override async Task<List<SettingValue>> GetAllAsync(SettingDefinition[] settings)
    {
        if (_currentWorkspace.Id == null)
        {
            return settings.Select(s => new SettingValue(s.Name, null)).ToList();
        }

        return await SettingStore.GetAllAsync(settings.Select(s => s.Name).ToArray(), Name, _currentWorkspace.Id.Value.ToString());
    }
}

/// <summary>Stores workspace-level setting values ("W"); a null provider key means the current workspace.</summary>
public class WorkspaceSettingManagementProvider : SettingManagementProvider, ITransientDependency
{
    private readonly ICurrentWorkspace _currentWorkspace;

    public WorkspaceSettingManagementProvider(ISettingManagementStore settingManagementStore, ICurrentWorkspace currentWorkspace)
        : base(settingManagementStore)
    {
        _currentWorkspace = currentWorkspace;
    }

    public override string Name => WorkspaceSettingValueProvider.ProviderName;

    protected override string? NormalizeProviderKey(string? providerKey)
    {
        return providerKey ?? _currentWorkspace.Id?.ToString();
    }
}
