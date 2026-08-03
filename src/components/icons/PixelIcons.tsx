import React from "react";

interface IconProps {
    size?: number;
    className?: string;
}

const Base: React.FC<{ size?: number; className?: string; children: React.ReactNode }> = ({ size = 14, className = "", children }) => (
    <svg width={size} height={size} viewBox="0 0 16 16" shapeRendering="crispEdges" className={className} fill="currentColor">
        {children}
    </svg>
);

export const PixelSyncIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={2} y={3} width={9} height={2} />
        <rect x={11} y={3} width={2} height={5} />
        <rect x={9} y={3} width={2} height={2} transform="translate(0,0)" />
        <rect x={11} y={2} width={2} height={2} />
        <rect x={5} y={11} width={9} height={2} />
        <rect x={3} y={8} width={2} height={5} />
        <rect x={3} y={12} width={2} height={2} />
    </Base>
);

export const PixelShieldIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={4} y={2} width={8} height={2} />
        <rect x={3} y={4} width={2} height={6} />
        <rect x={11} y={4} width={2} height={6} />
        <rect x={5} y={4} width={6} height={6} />
        <rect x={4} y={10} width={2} height={2} />
        <rect x={10} y={10} width={2} height={2} />
        <rect x={6} y={12} width={4} height={2} />
    </Base>
);

export const PixelTrashIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={3} y={4} width={10} height={2} />
        <rect x={6} y={2} width={4} height={2} />
        <rect x={4} y={6} width={2} height={8} />
        <rect x={7} y={6} width={2} height={8} />
        <rect x={10} y={6} width={2} height={8} />
    </Base>
);

export const PixelWalletIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={2} y={4} width={12} height={9} />
        <rect x={2} y={2} width={9} height={2} fillOpacity={0.6} />
        <rect x={10} y={7} width={4} height={4} fill="#f5e6bf" fillOpacity={0.9} />
        <rect x={11} y={8} width={2} height={2} fill="currentColor" />
    </Base>
);

export const PixelGearIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={6} y={1} width={4} height={2} />
        <rect x={6} y={13} width={4} height={2} />
        <rect x={1} y={6} width={2} height={4} />
        <rect x={13} y={6} width={2} height={4} />
        <rect x={3} y={3} width={2} height={2} />
        <rect x={11} y={3} width={2} height={2} />
        <rect x={3} y={11} width={2} height={2} />
        <rect x={11} y={11} width={2} height={2} />
        <rect x={5} y={5} width={6} height={6} />
    </Base>
);

export const PixelBoltIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={8} y={1} width={4} height={2} />
        <rect x={6} y={3} width={4} height={2} />
        <rect x={4} y={5} width={4} height={2} />
        <rect x={2} y={7} width={4} height={2} />
        <rect x={6} y={7} width={4} height={2} />
        <rect x={8} y={9} width={4} height={2} />
        <rect x={6} y={11} width={4} height={2} />
        <rect x={4} y={13} width={4} height={2} />
    </Base>
);

export const PixelLockIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={5} y={2} width={6} height={2} />
        <rect x={4} y={4} width={2} height={3} />
        <rect x={10} y={4} width={2} height={3} />
        <rect x={3} y={7} width={10} height={7} />
        <rect x={7} y={9} width={2} height={3} fill="#f5e6bf" fillOpacity={0.8} />
    </Base>
);

export const PixelPlusIcon: React.FC<IconProps> = (props) => (
    <Base {...props}>
        <rect x={7} y={2} width={2} height={12} />
        <rect x={2} y={7} width={12} height={2} />
    </Base>
);
