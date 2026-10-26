export class Checklist {
  #items;
  constructor(saved = '[]') {
    const items = JSON.parse(saved);
    if (!Array.isArray(items) || items.some(item => typeof item?.id !== 'string' || typeof item?.title !== 'string' || typeof item?.done !== 'boolean') || new Set(items.map(item => item.id)).size !== items.length) throw new TypeError('Invalid checklist');
    this.#items = structuredClone(items);
  }
  add(id, title) {
    if (typeof id !== 'string' || !id.trim() || typeof title !== 'string' || !title.trim()) throw new TypeError('ID and title required');
    if (this.#items.some(item => item.id === id)) throw new Error('Duplicate ID');
    this.#items.push({ id, title: title.trim(), done: false });
  }
  complete(id) {
    const item = this.#items.find(item => item.id === id);
    if (!item) throw new Error('Unknown ID');
    item.done = true;
  }
  list() { return structuredClone(this.#items); }
  save() { return JSON.stringify(this.#items); }
}