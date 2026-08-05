import { type OpenClawPluginApi, type OpenClawPluginCommandDefinition, type OpenClawPluginDefinition, type PluginCommandContext, type PluginCommandResult } from "openclaw/plugin-sdk/plugin-entry";
type RegisterCommandApi = Pick<OpenClawPluginApi, "registerCommand">;
export declare function createLatticeCommandDefinition(): OpenClawPluginCommandDefinition;
export declare function registerLatticeCommand(api: RegisterCommandApi): void;
export declare function handleLatticeCommand(context: PluginCommandContext): Promise<PluginCommandResult>;
declare const latticePlugin: OpenClawPluginDefinition;
export default latticePlugin;
//# sourceMappingURL=index.d.ts.map