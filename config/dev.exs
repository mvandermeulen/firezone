import Config

# Configure your database
config :fz_http, FzHttp.Repo,
  username: "postgres",
  password: "postgres",
  database: "firezone_dev",
  hostname: "localhost",
  show_sensitive_data_on_connection_error: true,
  pool_size: 10

# For development, we disable any cache and enable
# debugging and code reloading.
#
# The watchers configuration can be used to run external
# watchers to your application. For example, we use it
# with webpack to recompile .js and .css sources.
config :fz_http, FzHttpWeb.Endpoint,
  http: [port: 4000],
  debug_errors: true,
  code_reloader: true,
  check_origin: false,
  watchers: [
    node: [
      "node_modules/webpack/bin/webpack.js",
      "--mode",
      "development",
      "--watch",
      "--watch-options-stdin",
      cd: Path.expand("../apps/fz_http/assets", __DIR__)
    ]
  ]

config :fz_vpn,
  cli: FzVpn.CLI.Live

config :fz_wall,
  cli: FzWall.CLI.Live

# ## SSL Support
#
# In order to use HTTPS in development, a self-signed
# certificate can be generated by running the following
# Mix task:
#
#     mix phx.gen.cert
#
# Note that this task requires Erlang/OTP 20 or later.
# Run `mix help phx.gen.cert` for more information.
#
# The `http:` config above can be replaced with:
#
#     https: [
#       port: 4001,
#       cipher_suite: :strong,
#       keyfile: "priv/cert/selfsigned_key.pem",
#       certfile: "priv/cert/selfsigned.pem"
#     ],
#
# If desired, both `http:` and `https:` keys can be
# configured to run both http and https servers on
# different ports.

# Watch static and templates for browser reloading.
config :fz_http, FzHttpWeb.Endpoint,
  secret_key_base: "5OVYJ83AcoQcPmdKNksuBhJFBhjHD1uUa9mDOHV/6EIdBQ6pXksIhkVeWIzFk5SD",
  live_view: [
    signing_salt: "t01wa0K4lUd7mKa0HAtZdE+jFOPDDejX"
  ],
  live_reload: [
    patterns: [
      ~r"apps/fz_http/priv/static/.*(js|css|png|jpeg|jpg|gif|svg)$",
      ~r"apps/fz_http/priv/gettext/.*(po)$",
      ~r"apps/fz_http/lib/fz_http_web/(live|views)/.*(ex)$",
      ~r"apps/fz_http/lib/fz_http_web/templates/.*(eex)$"
    ]
  ]

# Do not include metadata nor timestamps in development logs
config :logger, :console, format: "[$level] $message\n"

# Set a higher stacktrace during development. Avoid configuring such
# in production as building large stacktraces may be expensive.
config :phoenix, :stacktrace_depth, 20

# Initialize plugs at runtime for faster development compilation
config :phoenix, :plug_init_mode, :runtime
config :fz_vpn, :server_process_opts, name: {:global, :fz_vpn_server}
config :fz_wall, :server_process_opts, name: {:global, :fz_wall_server}

config(
  :fz_http,
  :disable_signup,
  case System.get_env("DISABLE_SIGNUP") do
    d when d in ["1", "yes"] -> true
    _ -> false
  end
)
