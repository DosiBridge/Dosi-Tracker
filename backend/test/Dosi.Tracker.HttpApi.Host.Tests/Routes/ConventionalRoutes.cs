using System;
using System.Reflection;
using Microsoft.AspNetCore.Mvc.ApplicationModels;
using Microsoft.Extensions.Options;
using Volo.Abp.Application.Services;
using Volo.Abp.AspNetCore.Mvc.Conventions;
using Volo.Abp.Http;

namespace Dosi.Tracker.Routes;

/// <summary>
/// The HTTP verb and route ABP's auto API controllers give an application service method (the "app" root path of the
/// host's conventional controllers), built by ABP's own <see cref="ConventionalRouteBuilder"/>: the verb comes from the
/// method name's prefix, a parameter named <c>id</c> becomes a route segment, and so does a method's only other
/// "...Id" parameter unless it is optional; everything else is a query parameter or the body.
/// </summary>
internal static class ConventionalRoutes
{
    public static (string Verb, string Route) Of(Type appServiceType, string methodName)
    {
        var method = appServiceType.GetMethod(methodName)!;
        var controller = new ControllerModel(appServiceType.GetTypeInfo(), appServiceType.GetCustomAttributes(true));
        // As MVC builds it: "Async" dropped from the action name, attributes (pseudo-attributes included) per parameter.
        var action = new ActionModel(method, method.GetCustomAttributes(true))
        {
            ActionName = methodName[..^"Async".Length],
            Controller = controller
        };
        foreach (var parameter in method.GetParameters())
        {
            action.Parameters.Add(new ParameterModel(parameter, parameter.GetCustomAttributes(true))
            {
                Action = action,
                ParameterName = parameter.Name!
            });
        }

        var verb = HttpMethodHelper.GetConventionalVerbForMethodName(action.ActionName);
        // "TeamAppService" is served by the "Team" controller, as ABP names conventional controllers.
        var controllerName = appServiceType.Name.RemovePostFix(ApplicationService.CommonPostfixes);
        var route = new ConventionalRouteBuilder(Options.Create(new AbpConventionalControllerOptions()))
            .Build("app", controllerName, action, verb, null);

        return (verb, route);
    }
}
