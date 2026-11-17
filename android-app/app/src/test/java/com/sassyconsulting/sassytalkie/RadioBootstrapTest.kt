// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
package com.sassyconsulting.sassytalkie

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RadioBootstrapTest {

    private val ready = RadioBootstrap.State(nativeReady = true, sessionActive = true, profileSet = true)

    @Test
    fun resumesWhenALaunchWouldGoStraightToTheRadio() {
        assertTrue(RadioBootstrap.mayResumeWithoutUi(micGranted = true, entitled = true, state = ready))
    }

    @Test
    fun staysDownWithoutASavedSession() {
        // The user ended the session: a launch would show Authenticate, so the
        // service must not reconnect anything on its own.
        assertFalse(
            RadioBootstrap.mayResumeWithoutUi(true, true, ready.copy(sessionActive = false)),
        )
    }

    @Test
    fun staysDownBeforeProfileSetup() {
        assertFalse(RadioBootstrap.mayResumeWithoutUi(true, true, ready.copy(profileSet = false)))
    }

    @Test
    fun staysDownWhenLockedOrWithoutTheMicrophone() {
        assertFalse(RadioBootstrap.mayResumeWithoutUi(micGranted = true, entitled = false, state = ready))
        assertFalse(RadioBootstrap.mayResumeWithoutUi(micGranted = false, entitled = true, state = ready))
    }

    @Test
    fun staysDownIfTheNativeLibraryDidNotStart() {
        assertFalse(RadioBootstrap.mayResumeWithoutUi(true, true, ready.copy(nativeReady = false)))
    }
}
