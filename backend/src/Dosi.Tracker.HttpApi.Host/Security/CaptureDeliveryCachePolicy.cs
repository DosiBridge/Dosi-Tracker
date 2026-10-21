using System;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;

namespace Dosi.Tracker.Security;

/// <summary>
/// Enforces an explicit non-storable cache policy (Cache-Control: no-store, private and Pragma: no-cache)
/// on private capture delivery routes (e.g. /api/app/activity/screenshot/{id}/content).
/// 
/// This guarantees that neither authenticated binary capture streams nor pre-controller/pipeline
/// error responses (401 unauthenticated, 400 bad workspace, 403 non-member, 404 not found) can ever
/// advertise public or max-age cacheability to intermediate caches or client storage.
/// </summary>
public class CaptureDeliveryCachePolicyMiddleware
{
    private readonly RequestDelegate _next;
    public static readonly PathString CapturePath = new("/api/app/activity/screenshot");

    public CaptureDeliveryCachePolicyMiddleware(RequestDelegate next)
    {
        _next = next;
    }

    public async Task InvokeAsync(HttpContext context)
    {
        if (context.Request.Path.StartsWithSegments(CapturePath, StringComparison.OrdinalIgnoreCase))
        {
            context.Response.OnStarting(() =>
            {
                context.Response.Headers.CacheControl = "no-store, private";
                context.Response.Headers.Pragma = "no-cache";
                return Task.CompletedTask;
            });
        }

        await _next(context);
    }
}

public static class CaptureDeliveryCachePolicyApplicationBuilderExtensions
{
    public static IApplicationBuilder UseCaptureDeliveryCachePolicy(this IApplicationBuilder app)
    {
        return app.UseMiddleware<CaptureDeliveryCachePolicyMiddleware>();
    }
}
