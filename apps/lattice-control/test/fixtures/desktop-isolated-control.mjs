import { writeFileSync } from "node:fs";
import { createLatticeServer } from "../../src/server.mjs";
import { defaultControlDatabasePath } from "../../src/database-path.mjs";

const readyPath = process.env.LATTICE_DESKTOP_CONTROL_READY;
if (!readyPath) {
  throw new Error("LATTICE_DESKTOP_CONTROL_READY is required");
}

const application = createLatticeServer({ databasePath: defaultControlDatabasePath() });
await new Promise((resolve, reject) => {
  application.server.once("error", reject);
  application.server.listen(0, "127.0.0.1", resolve);
});
const address = application.server.address();
if (!address || typeof address === "string") {
  throw new Error("isolated Control did not bind TCP");
}
writeFileSync(readyPath, `${address.port}\n`, { encoding: "utf8", flag: "wx" });
