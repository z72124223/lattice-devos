export class ControlWorkService {
  constructor({ store }) {
    if (!store || typeof store.getWorkSnapshot !== "function") {
      throw new TypeError("Control work service requires the LATTICE Control store");
    }
    this.store = store;
  }

  setWorkRelations(input) {
    return this.store.setWorkRelations(input);
  }

  workSnapshot(input) {
    return this.store.getWorkSnapshot(input);
  }

  workNode(input) {
    return this.store.getWorkNode(input);
  }
}
