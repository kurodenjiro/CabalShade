// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC721/ERC721.sol";

/// @notice Redeemable digital vouchers (e.g. AI compute credit, relay bandwidth
/// credit). Minting assigns the token to msg.sender, which is itself the
/// proof-of-possession check: nobody can mint a voucher into someone else's
/// name. Redeeming requires being the current owner and burns the token.
contract CabalMeshVoucher is ERC721 {
    struct VoucherData {
        string voucherType;
        string description;
        address mintedBy;
    }

    uint256 public nextTokenId = 1;
    mapping(uint256 => VoucherData) public vouchers;

    event VoucherMinted(uint256 indexed tokenId, address indexed owner, string voucherType, string description);
    event VoucherRedeemed(uint256 indexed tokenId, address indexed redeemer, string voucherType);

    constructor() ERC721("CabalMesh Voucher", "CMV") {}

    function mintVoucher(string calldata voucherType, string calldata description) external returns (uint256) {
        require(bytes(voucherType).length > 0, "Voucher type required");

        uint256 tokenId = nextTokenId++;
        _safeMint(msg.sender, tokenId);
        vouchers[tokenId] = VoucherData({
            voucherType: voucherType,
            description: description,
            mintedBy: msg.sender
        });

        emit VoucherMinted(tokenId, msg.sender, voucherType, description);
        return tokenId;
    }

    function redeemVoucher(uint256 tokenId) external {
        require(ownerOf(tokenId) == msg.sender, "Not the owner");

        string memory vType = vouchers[tokenId].voucherType;
        _burn(tokenId);

        emit VoucherRedeemed(tokenId, msg.sender, vType);
    }
}
