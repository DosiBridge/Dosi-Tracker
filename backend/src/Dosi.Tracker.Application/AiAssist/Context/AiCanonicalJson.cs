using System;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Dosi.Tracker.AiAssist.Context;

/// <summary>
/// The one way a bundle becomes text: object keys sorted (ordinal), arrays in order, no indentation, empty sections
/// left out. The same bundle therefore always gives the same bytes, so its SHA-256 identifies the data in the result
/// cache. The resolution (real names) is never part of it.
/// </summary>
public static class AiCanonicalJson
{
    // The text goes to a model, never into HTML, so non-ASCII letters stay readable instead of \u-escaped. Quoted text
    // has no '<' or '>' left (UntrustedText), and quotes and backslashes are always escaped.
    private static readonly JsonWriterOptions WriterOptions = new()
    {
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
        Indented = false
    };

    /// <summary>The node with every object's keys sorted, compact; "null" for null.</summary>
    public static string Serialize(JsonNode? node)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream, WriterOptions))
        {
            Write(writer, node);
        }

        return Encoding.UTF8.GetString(stream.ToArray());
    }

    /// <summary>The data block: facts, lists, caveats, routes and candidates.</summary>
    public static string Data(AiContextBundle bundle)
    {
        var data = new JsonObject();

        if (bundle.FactList.Count > 0)
        {
            var facts = new JsonObject();
            foreach (var (id, fact) in bundle.FactList)
            {
                facts[id] = new JsonObject { ["label"] = fact.Label, ["display"] = fact.Display };
            }

            data["facts"] = facts;
        }

        if (bundle.Lists.Count > 0)
        {
            data["lists"] = bundle.Lists.DeepClone();
        }

        if (bundle.Caveats.Count > 0)
        {
            data["caveats"] = new JsonArray(bundle.Caveats.Select(c => (JsonNode?)JsonValue.Create(c)).ToArray());
        }

        if (bundle.Routes.Count > 0)
        {
            data["routes"] = new JsonArray(bundle.Routes.Select(r => (JsonNode?)JsonValue.Create(r)).ToArray());
        }

        if (bundle.Candidates.Count > 0)
        {
            data["candidates"] = bundle.Candidates.DeepClone();
        }

        return Serialize(data);
    }

    /// <summary>The untrusted block: id → sanitized quoted text.</summary>
    public static string Labels(AiContextBundle bundle)
    {
        var labels = new JsonObject();
        foreach (var (id, text) in bundle.LabelList)
        {
            labels[id] = text;
        }

        return Serialize(labels);
    }

    /// <summary>SHA-256 (lowercase hex) of the data and labels blocks: equal for equal data, different when any fact,
    /// list, label, caveat, route or candidate differs.</summary>
    public static string Hash(AiContextBundle bundle)
    {
        return Sha256Hex(Data(bundle) + "\n" + Labels(bundle));
    }

    /// <summary>SHA-256 of the UTF-8 text, as lowercase hex.</summary>
    public static string Sha256Hex(string text)
    {
        return Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(text)));
    }

    private static void Write(Utf8JsonWriter writer, JsonNode? node)
    {
        switch (node)
        {
            case null:
                writer.WriteNullValue();
                break;
            case JsonObject obj:
                writer.WriteStartObject();
                foreach (var (key, value) in obj.OrderBy(p => p.Key, StringComparer.Ordinal))
                {
                    writer.WritePropertyName(key);
                    Write(writer, value);
                }

                writer.WriteEndObject();
                break;
            case JsonArray array:
                writer.WriteStartArray();
                foreach (var item in array)
                {
                    Write(writer, item);
                }

                writer.WriteEndArray();
                break;
            default:
                node.WriteTo(writer);
                break;
        }
    }
}
