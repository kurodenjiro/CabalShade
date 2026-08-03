import { expect } from "chai";
import { ethers } from "hardhat";
import { CabalMeshVoucher } from "../typechain-types";
import { HardhatEthersSigner } from "@nomicfoundation/hardhat-ethers/signers";

describe("CabalMeshVoucher", function () {
    let voucher: CabalMeshVoucher;
    let owner: HardhatEthersSigner;
    let other: HardhatEthersSigner;

    beforeEach(async function () {
        [owner, other] = await ethers.getSigners();
        const Voucher = await ethers.getContractFactory("CabalMeshVoucher");
        voucher = await Voucher.deploy();
        await voucher.waitForDeployment();
    });

    it("mints a voucher to the caller", async function () {
        await expect(voucher.connect(owner).mintVoucher("AI Compute Credit", "1 hour Ollama compute"))
            .to.emit(voucher, "VoucherMinted")
            .withArgs(1, owner.address, "AI Compute Credit", "1 hour Ollama compute");

        expect(await voucher.ownerOf(1)).to.equal(owner.address);
        const data = await voucher.vouchers(1);
        expect(data.voucherType).to.equal("AI Compute Credit");
        expect(data.description).to.equal("1 hour Ollama compute");
        expect(data.mintedBy).to.equal(owner.address);
    });

    it("reverts minting with an empty voucher type", async function () {
        await expect(
            voucher.connect(owner).mintVoucher("", "some description")
        ).to.be.revertedWith("Voucher type required");
    });

    it("allows the owner to redeem (burn) their voucher", async function () {
        await voucher.connect(owner).mintVoucher("Relay Bandwidth Credit", "500MB");

        await expect(voucher.connect(owner).redeemVoucher(1))
            .to.emit(voucher, "VoucherRedeemed")
            .withArgs(1, owner.address, "Relay Bandwidth Credit");

        await expect(voucher.ownerOf(1)).to.be.reverted;
    });

    it("reverts redemption by a non-owner", async function () {
        await voucher.connect(owner).mintVoucher("Relay Bandwidth Credit", "500MB");

        await expect(
            voucher.connect(other).redeemVoucher(1)
        ).to.be.revertedWith("Not the owner");
    });

    it("reverts redeeming the same voucher twice", async function () {
        await voucher.connect(owner).mintVoucher("Relay Bandwidth Credit", "500MB");
        await voucher.connect(owner).redeemVoucher(1);

        await expect(
            voucher.connect(owner).redeemVoucher(1)
        ).to.be.reverted;
    });
});
