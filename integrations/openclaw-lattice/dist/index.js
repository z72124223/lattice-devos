import { definePluginEntry, } from "openclaw/plugin-sdk/plugin-entry";
import { deriveCommandIdentities, LatticeInputError, parseLatticeArguments, } from "./commands.js";
import { LatticeLaunchError, readLatticeLaunchEnvironment, } from "./official.js";
import { exchangeAuthenticatedFrames, LatticeTransportError, } from "./transport.js";
import { decodeCommandReply, encodeClientHello, encodeCommandRequest, LatticeWireError, } from "./wire.js";
const USAGE = "Usage: /lattice status project <project_id> | status command <project_id> <command_id> | status task <project_id> <snapshot_id> <task_id> <revision> <task_digest> <ledger_digest> | submit <task_digest> | stop <project_id> <snapshot_id> <task_id> <revision> <task_digest> <ledger_digest> <attempt_id> <USER_REQUESTED|SUPERSEDED|SAFETY_CONCERN>";
export function createLatticeCommandDefinition() {
    return {
        acceptsArgs: true,
        description: "Send one closed typed command to the local LATTICE gateway",
        handler: handleLatticeCommand,
        name: "lattice",
        requireAuth: true,
    };
}
export function registerLatticeCommand(api) {
    api.registerCommand(createLatticeCommandDefinition());
}
export async function handleLatticeCommand(context) {
    if (!context.isAuthorizedSender || context.sessionKey === undefined) {
        return textReply("LATTICE authorization or stable session binding is unavailable");
    }
    let command;
    try {
        command = parseLatticeArguments(context.args ?? "");
    }
    catch (error) {
        if (error instanceof LatticeInputError) {
            return textReply(USAGE);
        }
        return textReply("LATTICE command rejected");
    }
    let identities;
    try {
        identities = deriveCommandIdentities(context.sessionKey, command);
    }
    catch {
        return textReply("LATTICE stable session binding is unavailable");
    }
    let launch;
    try {
        launch = readLatticeLaunchEnvironment();
    }
    catch (error) {
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
    }
    catch (error) {
        if (command.action === "submit" &&
            error instanceof LatticeTransportError &&
            (error.code === "TIMEOUT" || error.code === "DISCONNECTED")) {
            return textReply(`LATTICE submit outcome unknown; do not resubmit automatically [command_id=${identities.commandId}]`);
        }
        if (error instanceof LatticeTransportError || error instanceof LatticeWireError) {
            return textReply(`LATTICE command failed closed [command_id=${identities.commandId}]`);
        }
        return textReply(`LATTICE command failed closed [command_id=${identities.commandId}]`);
    }
    finally {
        launch.rootKey.fill(0);
    }
}
function textReply(text) {
    return { text };
}
const latticePlugin = definePluginEntry({
    id: "lattice-devos",
    name: "LATTICE DevOS",
    description: "Authenticated loopback-only LATTICE gateway command entry",
    register(api) {
        registerLatticeCommand(api);
    },
});
export default latticePlugin;
//# sourceMappingURL=index.js.map