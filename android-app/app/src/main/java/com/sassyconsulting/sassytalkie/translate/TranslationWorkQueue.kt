// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie.translate

/**
 * Conflates partial hypotheses while retaining every final. Consumers always
 * drain finals first, and a single consumer serializes ML Kit access.
 */
class TranslationWorkQueue {
    data class Work(val text: String, val isFinal: Boolean)

    private val finals = ArrayDeque<Work>()
    private var partial: Work? = null

    @Synchronized
    fun offer(text: String, isFinal: Boolean) {
        val work = Work(text, isFinal)
        if (isFinal) {
            partial = null
            finals.addLast(work)
        } else {
            partial = work
        }
    }

    @Synchronized
    fun poll(): Work? = if (finals.isNotEmpty()) finals.removeFirst() else partial.also { partial = null }

    @Synchronized
    fun clearPartial() {
        partial = null
    }

    @Synchronized
    fun clear() {
        finals.clear()
        partial = null
    }
}
