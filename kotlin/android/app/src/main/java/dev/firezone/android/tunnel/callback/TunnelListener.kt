package dev.firezone.android.tunnel.callback

import dev.firezone.android.tunnel.model.Resource
import dev.firezone.android.tunnel.model.Tunnel

interface TunnelListener {

    fun onTunnelStateUpdate(state: Tunnel.State)

    fun onResourcesUpdate(resources: List<Resource>)

    fun onError(error: String): Boolean
}