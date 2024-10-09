If you have an Aranet4 and enable "smart home integration" on it, what that actually means is that it'll broadcast every reading (CO₂, temperature, humidity, atmospheric pressure) as Manufacturer Data over Bluetooth Low Energy.

This is a small program that listens for those readings and writes them out as a stream of JSON objects, for all your scripting needs:

```
$ cargo run --
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.14s
     Running `target/debug/arara`
{
  "aranet4": {
    "Aranet4 26837": {
      "co2": 1967,
      "temperature": 22.6,
      "pressure": 992.7,
      "humidity": 59,
      "battery": 33,
      "status": 3
    }
  }
}
{
  "aranet4": {
    "Aranet4 26826": {
      "co2": 628,
      "temperature": 23.5,
      "pressure": 992.7,
      "humidity": 55,
      "battery": 26,
      "status": 1
    },
    "Aranet4 26837": {
      "co2": 1967,
      "temperature": 22.6,
      "pressure": 992.7,
      "humidity": 59,
      "battery": 33,
      "status": 3
    }
  }
}
```

Or as a [custom waybar module](https://github.com/Alexays/Waybar/wiki/Module:-Custom) - run `cargo build` to build the binary, then add this to your `~/.config/waybar/config`:
```json
{
    "modules-right": ["custom/arara"],
    "custom/arara": {
        "exec": "$HOME/src/embr/arara/target/debug/arara -F waybar",
        "return-type": "json",
    }
}
```

The module looks like this: "🪟 609 🌡️ 23.50 ☔ 55 🗜️ 993", and the tooltip lists all the aranets that have been seen lately.
