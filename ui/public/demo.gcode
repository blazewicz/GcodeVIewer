; GcodeWelder renderer demo: rapids, feeds, arcs, and a cubic spline.
G21
G90
M82
G92 E0
G1 Z0.2 F600

; full circle drawn clockwise
G0 X-12 Y20 F3000
G1 F1200
G2 X-12 Y20 I-12 J0 E9.0478

; two arcs forming a circle at (24, 20)
G0 X8 Y20
G2 X40 Y20 I16 J0 E33.0919
G2 X8 Y20 I-16 J0 E57.1360

; cubic Bezier S-curve
G0 X-42 Y-28
G5 X-6 Y-12 I0 J16 P0 Q-16 E80.9751

; polyline
G0 X10 Y-32
G1 X20 Y-12 E88.4899
G1 X30 Y-32 E95.9475
G1 X40 Y-12 E103.4051

; helical full circle rising 1 mm
G0 X0 Y0
G2 X0 Y0 Z1.2 I0 J6 E116.0991
