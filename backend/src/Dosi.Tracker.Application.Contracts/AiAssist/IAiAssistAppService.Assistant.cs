using System.Threading.Tasks;

namespace Dosi.Tracker.AiAssist;

/// <summary>Ask AI: a conversational, tool-calling, read-only assistant over this workspace's tracked data.</summary>
public partial interface IAiAssistAppService
{
    /// <summary>POST api/app/ai-assist/ask.</summary>
    Task<AiAnswerDto> AskAsync(AiAskInput input);
}
