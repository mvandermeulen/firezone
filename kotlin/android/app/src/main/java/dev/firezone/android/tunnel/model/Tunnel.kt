package dev.firezone.android.tunnel.model

import android.os.Parcelable
import com.squareup.moshi.JsonClass
import kotlinx.parcelize.Parcelize

@JsonClass(generateAdapter = true)
@Parcelize
data class Tunnel(
    val config: TunnelConfig = TunnelConfig(),
    var state: State = State.Down,
    val routes: List<String> = emptyList(),
    val resources: List<Resource> = emptyList(),
): Parcelable {

    enum class State {
        Up, Connecting, Down;
    }
}