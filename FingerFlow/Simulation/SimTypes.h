//
//  SimTypes.h
//  FingerFlow
//
//  Types and constants shared between Swift (via the bridging header)
//  and the Metal shaders. Keep every field 4 bytes wide so the struct
//  layout is identical on both sides with no padding.
//

#ifndef SimTypes_h
#define SimTypes_h

// Cell types stored in the cellType grid (one byte per cell).
#define CELL_FLUID  0
#define CELL_WALL   1
#define CELL_INLET  2
#define CELL_OUTLET 3

// Render modes for the field visualisation kernel.
#define RENDER_DYE       0
#define RENDER_SPEED     1
#define RENDER_VORTICITY 2
#define RENDER_PRESSURE  3

typedef struct {
    int   width;        // grid cells in x
    int   height;       // grid cells in y
    float omega;        // BGK relaxation rate, 1 / tau, tau = 3*nu + 0.5
    float inletSpeed;   // lattice speed applied at fan/inlet cells
    float dyeDt;        // lattice time advanced this frame (dye advection)
    float dyeDecay;     // per-frame dye retention multiplier
    int   renderMode;   // RENDER_* constant
} SimParams;

#endif /* SimTypes_h */
