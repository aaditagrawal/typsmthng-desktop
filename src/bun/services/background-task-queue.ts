export class BackgroundTaskQueue {
  private tail: Promise<void> = Promise.resolve();

  enqueue(task: () => Promise<void> | void): Promise<void> {
    const run = async () => {
      await task();
    };

    this.tail = this.tail.then(run, run);
    return this.tail;
  }

  async drain(): Promise<void> {
    await this.tail;
  }
}
