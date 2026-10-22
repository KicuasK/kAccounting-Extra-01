export class LatestRequestGate {
  private current: AbortController | null = null
  private cancelInProgress = false

  begin(): AbortController {
    const controller = new AbortController()
    if (this.cancelInProgress) {
      controller.abort()
      return controller
    }

    const previous = this.current
    this.current = controller
    // Abort listeners run synchronously. Publish the replacement first so a
    // retry started by a listener can supersede it without becoming orphaned.
    previous?.abort()
    return controller
  }

  /** Check immediately before publishing either a result or an error. */
  isCurrent(controller: AbortController): boolean {
    return this.current === controller && !controller.signal.aborted
  }

  /** Release ownership and report whether this request owned final cleanup. */
  finish(controller: AbortController): boolean {
    if (this.current !== controller) return false
    this.current = null
    return true
  }

  cancel(): void {
    if (this.cancelInProgress) return

    const current = this.current
    // Clear ownership and reject synchronous retries while abort listeners run.
    this.current = null
    this.cancelInProgress = true
    try {
      current?.abort()
    } finally {
      this.cancelInProgress = false
    }
  }
}
