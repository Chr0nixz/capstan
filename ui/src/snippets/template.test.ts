import { describe, expect, it } from "vitest";
import { renderSnippetTemplate, templateVariables, withAutoEnter } from "./template";

describe("snippet templates", () => {
  it("collects each variable once and preserves its default without mutating the template", () => {
    const template = "kubectl get pods -n {{namespace:default}} --context {{context}} {{namespace:default}}";
    expect(templateVariables(template)).toEqual([
      { name: "namespace", defaultValue: "default" },
      { name: "context", defaultValue: null },
    ]);
    expect(renderSnippetTemplate(template, { namespace: "staging", context: "work" }))
      .toBe("kubectl get pods -n staging --context work staging");
    expect(template).toContain("{{namespace:default}}");
  });

  it("allows empty variable values and reports unresolved inputs", () => {
    expect(renderSnippetTemplate("echo '{{value}}'", { value: "" })).toBe("echo ''");
    expect(() => renderSnippetTemplate("echo {{value}}", {})).toThrow("snippet_variable_missing");
  });

  it("supports Chinese names, Unicode identifiers and defaults without changing user values", () => {
    const template = "cd {{目录:/tmp}}; echo {{目录}} {{环境}} {{cafe\u0301:中文}} {{_选项2:}}";
    expect(templateVariables(template)).toEqual([
      { name: "目录", defaultValue: "/tmp" },
      { name: "环境", defaultValue: null },
      { name: "cafe\u0301", defaultValue: "中文" },
      { name: "_选项2", defaultValue: "" },
    ]);
    expect(renderSnippetTemplate(template, { 目录: "/srv/项目", 环境: "测试", "cafe\u0301": "值{{保持}}", _选项2: "" }))
      .toBe("cd /srv/项目; echo /srv/项目 测试 值{{保持}} ");
    expect(templateVariables("echo {{路径:https://example.test/a:b}}"))
      .toEqual([{ name: "路径", defaultValue: "https://example.test/a:b" }]);
  });

  it("sends double-brace text that is not a variable as written", () => {
    for (const template of [
      "{{目录", "目录}}", "{{}}", "{{:默认值}}", "{{two words}}", "{{1目录}}", "{{目录:{嵌套}}}",
      "docker ps --format '{{.Names}}'", "kubectl get pods -o go-template='{{range .items}}{{.metadata.name}}{{\"\\n\"}}'",
    ]) {
      expect(templateVariables(template)).toEqual([]);
      expect(renderSnippetTemplate(template, {})).toBe(template);
    }
  });

  it("fills variables next to literal Go template braces", () => {
    const template = "docker inspect -f '{{.State.Status}} {{ .Name }}' {{容器:web}}";
    expect(templateVariables(template)).toEqual([{ name: "容器", defaultValue: "web" }]);
    expect(renderSnippetTemplate(template, { 容器: "db" })).toBe("docker inspect -f '{{.State.Status}} {{ .Name }}' db");
  });

  it("requires own variable values rather than reading object prototypes", () => {
    for (const name of ["constructor", "__proto__", "toString"]) {
      expect(() => renderSnippetTemplate(`echo {{${name}}}`, {})).toThrow("snippet_variable_missing");
      expect(renderSnippetTemplate(`echo {{${name}}}`, Object.fromEntries([[name, "safe value"]]))).toBe("echo safe value");
    }
  });

  it("adds one Enter only when enabled and avoids a duplicate trailing line break", () => {
    expect(withAutoEnter("echo ready", true)).toBe("echo ready\r");
    expect(withAutoEnter("echo ready", false)).toBe("echo ready");
    expect(withAutoEnter("echo ready\n", true)).toBe("echo ready\n");
  });
});
