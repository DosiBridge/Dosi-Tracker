using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Assistant;
using Volo.Abp.Uow;

namespace Dosi.Tracker.AiAssist;

/// <summary>Ask AI: a conversational, tool-calling assistant. Delegates to <see cref="AiAssistantRunner"/>, which
/// owns every check, limit, tool call and the conversation; this app service only wires the request in.</summary>
public partial class AiAssistAppService
{
    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiAnswerDto> AskAsync(AiAskInput input) =>
        LazyServiceProvider.LazyGetRequiredService<AiAssistantRunner>().AskAsync(input);
}
