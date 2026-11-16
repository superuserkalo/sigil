import {
  Agent,
  AnthropicProvider,
  ChatCompletionsProvider,
  OpenAIProvider,
  pause,
} from "tiny-agent-sdk";
import type { AfterTool, BeforeTool } from "tiny-agent-sdk";
import type { HostConfig } from "./config.ts";
import type { DahRsCli } from "./dah-rs.ts";
import { SYSTEM_PROMPT } from "./prompt.ts";
import { redactSecrets } from "./redact.ts";
import { createDeadlockTools } from "./tools.ts";

/** Deploy, launch, install, gameinfo repair and publish require app approval. */
const APPROVAL_TOOL = /(deploy|publish|launch|install|gameinfo)/i;

const beforeTool: BeforeTool = (call) => {
  if (!APPROVAL_TOOL.test(call.name)) return undefined;
  return pause({
    kind: "approval",
    tool: call.name,
    arguments: call.arguments,
  });
};

const afterTool: AfterTool = (_call, result) => {
  if (typeof result.output !== "string") return undefined;
  const redacted = redactSecrets(result.output);
  return redacted === result.output ? undefined : redacted;
};

function providerFor(config: HostConfig) {
  const shared = {
    ...(config.apiKey === undefined ? {} : { apiKey: config.apiKey }),
    ...(config.baseUrl === undefined ? {} : { baseUrl: config.baseUrl }),
  };

  switch (config.provider) {
    case "anthropic":
      return new AnthropicProvider(shared);
    case "chat":
      return new ChatCompletionsProvider(shared);
    case "openai":
      return new OpenAIProvider(shared);
  }
}

export function createAgent(config: HostConfig, cli: DahRsCli): Agent {
  return new Agent({
    model: config.model,
    instructions: SYSTEM_PROMPT,
    tools: createDeadlockTools({ cli }),
    provider: providerFor(config),
    beforeTool,
    afterTool,
  });
}
