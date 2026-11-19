### The pairing dialog's fleet switch: admit the phone approved here to
### this computer's fleet when it asks to join.

remote-fleet-join = Also add to your fleet ({ $computers ->
    [one] one computer
   *[other] { $computers } computers
}, { $hosts ->
    [one] one host
   *[other] { $hosts } hosts
})
remote-fleet-join-note = The phone may then reach every Sheprd host in the fleet, and approve computers joining it. Remove it any time in Settings > Fleet.
