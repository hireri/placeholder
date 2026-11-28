package dev.metrolist.desktop

import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit
import kotlin.random.Random

/** Spaces request starts (the first [burst] after a quiet spell go at once) and backs the whole gate off after a failure. */
class RateGate(
    permits: Int,
    private val spacingMs: Long,
    private val backoffBaseMs: Long = 1500,
    private val maxBackoffMs: Long = 60_000,
    private val burst: Int = 1,
) {
    private val permits = Semaphore(permits)
    private val lock = Mutex()
    private var nextStart = 0L
    private var pausedUntil = 0L
    private var failures = 0

    suspend fun <T> run(block: suspend () -> Result<T>): Result<T> = permits.withPermit {
        val start = lock.withLock {
            val now = System.currentTimeMillis()
            val at = maxOf(now, nextStart - (burst - 1) * spacingMs, pausedUntil)
            nextStart = maxOf(nextStart, at) + spacingMs
            at
        }
        val wait = start - System.currentTimeMillis()
        if (wait > 0) delay(wait)

        val result = block()
        lock.withLock {
            if (result.isSuccess) {
                failures = 0
            } else {
                failures++
                val base = (backoffBaseMs shl failures.coerceAtMost(5)).coerceAtMost(maxBackoffMs)
                val jitter = (base * (Random.nextDouble(-0.2, 0.2))).toLong()
                pausedUntil = maxOf(pausedUntil, System.currentTimeMillis() + (base + jitter).coerceAtLeast(0))
            }
        }
        result
    }
}
