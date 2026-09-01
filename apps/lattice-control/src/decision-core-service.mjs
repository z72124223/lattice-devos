export class ControlDecisionService {
  constructor({ store }) {
    if (
      !store
      || typeof store.recordDecision !== "function"
      || typeof store.getCurrentDecisionsPacket !== "function"
      || typeof store.readDecision !== "function"
      || typeof store.searchDecisions !== "function"
    ) {
      throw new TypeError("Control decision service requires the LATTICE Control store");
    }
    this.store = store;
  }

  record(input) {
    return this.store.recordDecision(input);
  }

  current(input) {
    return this.store.getCurrentDecisionsPacket(input);
  }

  read(input) {
    return this.store.readDecision(input);
  }

  search(input) {
    return this.store.searchDecisions(input);
  }
}
