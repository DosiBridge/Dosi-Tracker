using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Prompts;

/// <summary><see cref="AiPromptSpec"/> (versions and the schema subset) and <see cref="AiSharedPolicy"/>.</summary>
public class AiPromptSpecTests
{
    private const string ValidSchema = """
        {"type":"object","additionalProperties":false,"required":["headline","note","items"],
         "properties":{
           "headline":{"type":"string"},
           "note":{"type":["string","null"]},
           "items":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["kind"],
             "properties":{"kind":{"type":"string","enum":["trend","coverage"]}}}}}}
        """;

    private static AiPromptSpec Spec(string schema = ValidSchema, int version = 1, string task = "Do it.",
        string instructions = "Rules.", string schemaName = "card", int? maxOutputTokens = null) =>
        new(AiFeatureIds.NoteSuggestions, version, task, instructions, AiPromptSpec.ParseSchema(schema), schemaName, maxOutputTokens);

    [Fact]
    public void A_Spec_Should_Carry_Its_Versioned_Prompt()
    {
        var spec = Spec(instructions: "  Line one.\r\nLine two.  ", task: "  Suggest a note.  ");

        spec.Feature.ShouldBe(AiFeatureIds.NoteSuggestions);
        spec.Version.ShouldBe("note-suggestions@1");
        spec.Task.ShouldBe("Suggest a note.");
        spec.Instructions.ShouldBe("Line one.\nLine two.");
        spec.SchemaName.ShouldBe("card");
        spec.Schema.GetProperty("type").GetString().ShouldBe("object");
        spec.MaxOutputTokens.ShouldBe(AiFeatureIds.NoteSuggestions.DefaultMaxOutputTokens);

        Spec(version: 3, maxOutputTokens: 321).Version.ShouldBe("note-suggestions@3");
        Spec(maxOutputTokens: 321).MaxOutputTokens.ShouldBe(321);
        Spec(maxOutputTokens: 0).MaxOutputTokens.ShouldBe(AiFeatureIds.NoteSuggestions.DefaultMaxOutputTokens);
    }

    [Fact]
    public void Malformed_Arguments_Should_Be_Refused()
    {
        Should.Throw<ArgumentException>(() => Spec(version: 0));
        Should.Throw<ArgumentException>(() => Spec(task: " "));
        Should.Throw<ArgumentException>(() => Spec(task: "two\nlines"));
        Should.Throw<ArgumentException>(() => Spec(instructions: ""));
        Should.Throw<ArgumentException>(() => Spec(schemaName: "has space"));
        Should.Throw<ArgumentException>(() => Spec(schemaName: new string('a', 65)));
        Should.Throw<ArgumentException>(() => Spec(schema: """{"type":"string"}"""));
    }

    [Fact]
    public void The_Supported_Subset_Should_Pass()
    {
        AiPromptSpec.SchemaViolations(AiPromptSpec.ParseSchema(ValidSchema)).ShouldBeEmpty();
    }

    [Theory]
    [InlineData("""[]""", "root")]
    [InlineData("""{"type":"array","items":{"type":"string"}}""", "root")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string","minLength":1}},"required":["a"],"additionalProperties":false}""", "'minLength'")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string"}},"required":["a"],"additionalProperties":false,"description":"x"}""", "'description'")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["a"],"additionalProperties":false}""", "required must name every property")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":false}""", "required must name every property")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]}""", "additionalProperties false")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string"}},"required":["a"],"additionalProperties":true}""", "additionalProperties false")]
    [InlineData("""{"type":"object","required":[],"additionalProperties":false}""", "needs properties")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"array"}},"required":["a"],"additionalProperties":false}""", "needs items")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string","enum":[1,2]}},"required":["a"],"additionalProperties":false}""", "enum must list strings")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"string","enum":[]}},"required":["a"],"additionalProperties":false}""", "enum must list strings")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"date"}},"required":["a"],"additionalProperties":false}""", "needs a type")]
    [InlineData("""{"type":"object","properties":{"a":{}},"required":["a"],"additionalProperties":false}""", "needs a type")]
    [InlineData("""{"type":"object","properties":{"a":true},"required":["a"],"additionalProperties":false}""", "must be a schema object")]
    [InlineData("""{"type":"object","properties":{"a":{"type":"object","properties":{"b":{"type":"string"}},"required":["b"]}},"required":["a"],"additionalProperties":false}""", "$.a: an object needs additionalProperties false")]
    public void Anything_Outside_The_Subset_Should_Be_Reported(string schema, string problem)
    {
        AiPromptSpec.SchemaViolations(AiPromptSpec.ParseSchema(schema)).ShouldContain(p => p.Contains(problem));
    }

    [Fact]
    public void The_Policy_Should_Be_Verbatim_With_The_Reader_Filled_In()
    {
        AiSharedPolicy.V1.ShouldStartWith("You are the reporting assistant inside Dosi-Tracker");
        AiSharedPolicy.V1.ShouldContain(AiSharedPolicy.AudiencePlaceholder);
        AiSharedPolicy.V1.ShouldNotContain("\r");
        AiSharedPolicy.V1.Split('\n').Length.ShouldBe(9);

        foreach (var (audience, reader) in AiSharedPolicy.Audiences)
        {
            var policy = AiSharedPolicy.For(audience);
            policy.ShouldContain("factual text for " + reader + ".");
            policy.ShouldNotContain(AiSharedPolicy.AudiencePlaceholder);
        }

        AiSharedPolicy.Audiences.Keys.ShouldBe(
            new[] { AiUsageRecord.SelfAudience, AiUsageRecord.MemberAudience, AiUsageRecord.TeamAudience, AiUsageRecord.ProjectAudience },
            ignoreOrder: true);
        Should.Throw<ArgumentException>(() => AiSharedPolicy.For("help"));
    }
}
