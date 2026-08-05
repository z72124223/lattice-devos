import {
  definePluginEntry,
  type OpenClawPluginApi,
  type OpenClawPluginCommandDefinition,
  type OpenClawPluginDefinition,
  type PluginCommandContext,
  type PluginCommandResult,
} from "openclaw/plugin-sdk/plugin-entry";

import {
  deriveCommandIdentities,
  LatticeInputError,
  parseLatticeArguments,
  type LatticeCommand,
} from "./commands.js";
import {
  LatticeLaunchError,
  readLatticeLaunchEnvironment,
  type LatticeLaunchEnvironment,
} from "./official.js";
import {
  exchangeAuthenticatedFrames,
  LatticeTransportError,
} from "./transport.js";
import {
  decodeCommandReply,
  encodeClientHello,
  encodeCommandRequest,
  LatticeWireError,
} from "./wire.js";

const USAGE =
  "Usage: /lattice status project <project_id> | status command <project_id> <command_id> | status task <project_id> <snapshot_id> <task_id> <revision> <task_digest> <ledger_digest> | submit <task_digest> | stop <project_id> <snapshot_id> <task_id> <revision> <task_digest> <ledger_digest> <attempt_id> <USER_REQUESTED|SUPERSEDED|SAFETY_CONCERN>";

type RegisterCommandApi = Pick<OpenClawPluginApi, "registerCommand">;

export function createLatticeCommandDefinition(): OpenClawPluginCommandDefinition {
  return {
    acceptsArgs: true,
    description: "Send one closed typed command to the local LATTICE gateway",
    handler: handleLatticeCommand,
    name: "lattice",
    requireAuth: true,
  };
}

export function registerLatticeCommand(api: RegisterCommandApi): void {
  api.registerCommand(createLatticeCommandDefinition());
}

export async function handleLatticeCommand(
  context: PluginCommandContext,
): Promise<PluginCommandResult> {
  if (!context.isAuthorizedSender || context.sessionKey === undefined) {
    return textReply("LATTICE authorization or stable session binding is unavailable");
  }

  let command: LatticeCommand;
  try {
    command = parseLatticeArguments(context.args ?? "");
  } catch (error: unknown) {
    if (error instanceof LatticeInputError) {
      return textReply(USAGE);
    }
    return textReply("LATTICE command rejected");
  }

  let identities: ReturnType<typeof deriveCommandIdentities>;
  try {
    identities = deriveCommandIdentities(context.sessionKey, command);
  } catch {
    return textReply("LATTICE stable session binding is unavailable");
  }
  let launch: LatticeLaunchEnvironment;
  try {
    launch = readLatticeLaunchEnvironment();
  } catch (error: unknown) {
    if (error instanceof LatticeLaunchError) {
      return textReply("LATTICE local launch record is unavailable");
    }
    return textReply("LATTICE local launch record is unavailable");
  }

  try {
    const payload = await exchangeAuthenticatedFrames({
      commandPayload: encodeCommandRequest(command, identities),
      deadlineMs: launch.deadlineMs,
      helloPayload: encodeClientHello(launch.launchRecordId, launch.processStartNonce),
      port: launch.port,
      rootKey: launch.rootKey,
    });
    const reply = decodeCommandReply(payload, command, identities);
    return textReply(`${reply.summary} [command_id=${identities.commandId}]`);
  } catch (error: unknown) {
    if (
      command.action === "submit" &&
      error instanceof LatticeTransportError &&
      (error.code === "TIMEOUT" || error.code === "DISCONNECTED")
    ) {
      return textReply(
        `LATTICE submit outcome unknown; do not resubmit automatically [command_id=${identities.commandId}]`,
      );
    }
    if (error instanceof LatticeTransportError || error instanceof LatticeWireError) {
      return textReply(`LATTICE command failed closed [command_id=${identities.commandId}]`);
    }
    return textReply(`LATTICE command failed closed [command_id=${identities.commandId}]`);
  } finally {
    launch.rootKey.fill(0);
  }
}

function textReply(text: string): PluginCommandResult {
  return { text };
}

const latticePlugin: OpenClawPluginDefinition = definePluginEntry({
  id: "lattice-devos",
  name: "LATTICE DevOS",
  description: "Authenticated loopback-only LATTICE gateway command entry",
  register(api) {
    registerLatticeCommand(api);
  },
});

export default latticePlugin;
